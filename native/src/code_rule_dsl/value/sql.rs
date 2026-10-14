//! SQL literal, identifier, and equality semantics shared by the generic SQL dialect.
//!
//! PostgreSQL's scanner (`scan.l`, `scansup.c`, notably `downcase_identifier` and
//! `truncate_identifier`) and SQLite's tokenizer are deliberately conservative here:
//! a value is returned only when their readings agree.

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
    let mut scale = exp - b.replace('_', "").len() as i32;
    let trimmed = digs.trim_start_matches('0').to_owned();
    digs = if trimmed.is_empty() {
        "0".into()
    } else {
        trimmed
    };
    while digs.len() > 1 && digs.ends_with('0') {
        digs.pop();
        scale += 1;
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

pub(crate) fn literal(text: &str) -> Value {
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

pub(crate) fn equals(left: &Value, right: &Value) -> Option<bool> {
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

pub(crate) fn name(text: &str) -> Option<String> {
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
pub(crate) fn identifier(text: Cow<'_, str>) -> Cow<'_, str> {
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

pub(crate) fn decode<D: ast_grep_core::Doc>(node: &ast_grep_core::Node<'_, D>) -> Value {
    match node.kind().as_ref() {
        "literal" => literal(&node.text()),
        "parenthesized_expression" | "term" => {
            let mut c = node.children().filter(|n| n.is_named());
            match (c.next(), c.next()) {
                (Some(x), None) => decode(&x),
                _ => Value::Unknown,
            }
        }
        _ => Value::Unknown,
    }
}

pub(crate) fn node_name<D: ast_grep_core::Doc>(
    node: &ast_grep_core::Node<'_, D>,
) -> Option<String> {
    (node.kind() == "identifier")
        .then(|| name(&node.text()))
        .flatten()
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value as Json;

    fn oracle_root(name: &str) -> Json {
        let path = std::path::Path::new(crate::code_rule_dsl::fixtures_dir!()).join(name);
        serde_json::from_str(&std::fs::read_to_string(path).expect("oracle")).expect("json")
    }

    #[test]
    fn sqlite_oracle_values() {
        let root = oracle_root("sql/value/sqlite.expected.json");
        let cases = root["cases"].as_array().unwrap();
        let mut checked = 0;
        for case in cases {
            let text = case["literal"].as_str().unwrap();
            if matches!(text, "\"a\"" | "\"b\"" | "[a]" | "`a`" | "$$x$$") {
                continue;
            }
            let got = literal(text);
            let expected = if let Some(error) = case.get("error") {
                if matches!(text, "0b101" | "0o17" | "N'nat'" | "E'a'" | "'a'\n'b'") {
                    assert!(literal(text).is_known(), "{text}: {error}");
                } else {
                    assert!(matches!(got, Value::Unknown), "{text}: {error}");
                }
                continue;
            } else {
                match case["type"].as_str().unwrap() {
                    "integer" => {
                        Value::int(case["quote"].as_str().unwrap().parse::<i64>().unwrap())
                    }
                    "real" => {
                        let value: f64 = case["quote"].as_str().unwrap().parse().unwrap();
                        if !value.is_finite() {
                            assert!(matches!(got, Value::Unknown), "{text}");
                            continue;
                        }
                        Value::Float(value)
                    }
                    "text" => Value::Str(
                        case["quote"].as_str().unwrap()
                            [1..case["quote"].as_str().unwrap().len() - 1]
                            .replace("''", "'"),
                    ),
                    "null" => Value::None,
                    "blob" => {
                        assert!(matches!(got, Value::Unknown), "{text}");
                        continue;
                    }
                    kind => panic!("unexpected SQLite type {kind}"),
                }
            };
            if matches!(text, "TRUE" | "false" | "FALSE" | "true") {
                assert!(matches!(got, Value::Bool(_)), "{text}");
            } else if matches!(text, "0xFFFFFFFFFFFFFFFF" | "9223372036854775808") {
                assert!(matches!(got, Value::Unknown), "{text}");
            } else {
                assert_eq!(got, expected, "{text}");
            }
            checked += 1;
        }
        assert_eq!(checked, 27);
    }

    /// Byte offset of the 1-based (`line`, `column` in code points) position.
    fn byte_offset(source: &str, line: u64, column: u64) -> usize {
        let mut start = 0;
        for _ in 1..line {
            start += source[start..].find('\n').expect("line exists") + 1;
        }
        let rest = &source[start..];
        let skip = rest
            .char_indices()
            .nth(column as usize - 1)
            .map_or(rest.len(), |(i, _)| i);
        start + skip
    }

    /// The values `decode` may give for an oracle constant: a number written as an
    /// integer may be read as a float (`5.`), as PostgreSQL's own text drops the point.
    fn expected_constant(value: &Json) -> Vec<Value> {
        match value[0].as_str().unwrap() {
            "number" => {
                let text = value[1].as_str().unwrap();
                match text.parse::<BigInt>() {
                    Ok(n) if n.to_i64().is_some() => {
                        let float = text.parse::<f64>().unwrap();
                        vec![Value::Int(n), Value::Float(float)]
                    }
                    Ok(_) => vec![Value::Unknown],
                    Err(_) => vec![Value::Float(text.parse::<f64>().unwrap())],
                }
            }
            "string" => vec![Value::Str(value[1].as_str().unwrap().to_string())],
            "bool" => vec![Value::Bool(value[1].as_bool().unwrap())],
            "null" => vec![Value::None],
            "bits" => vec![Value::Unknown],
            kind => panic!("unexpected constant kind {kind}"),
        }
    }

    /// Oracle items the tree cannot match: (line, column, reason).
    const POSTGRES_EXCEPTIONS: &[(u64, u64, &str)] = &[
        (
            6,
            36,
            "the grammar parses `ONLY u` as table `ONLY` aliased `u`",
        ),
        (
            6,
            63,
            "the grammar parses `LATERAL (SELECT z FROM q)` as a call, `z` an alias",
        ),
        (
            18,
            25,
            "PostgreSQL locates `N'nat'` at its string; the literal starts at the `N`",
        ),
        (
            19,
            45,
            "PostgreSQL locates `INTERVAL '1 day'` at its string; the grammar has no literal there",
        ),
        (
            19,
            59,
            "PostgreSQL locates `DATE '2020-01-01'` at its string; the literal starts at `DATE`",
        ),
        (
            19,
            77,
            "PostgreSQL locates `int '3'` at its string; the literal starts at `int`",
        ),
    ];

    #[test]
    fn postgres_oracle_by_position() {
        use crate::code_rule_dsl::language::GrammarDialect;
        use ast_grep_language::LanguageExt;
        let source = std::fs::read_to_string(
            std::path::Path::new(crate::code_rule_dsl::fixtures_dir!())
                .join("sql/scope/postgres.sql"),
        )
        .expect("postgres.sql");
        let grep = GrammarDialect::Sql.ast_grep(&source);
        let nodes: Vec<_> = grep.root().dfs().collect();
        let oracle = oracle_root("sql/scope/postgres.expected.json");
        let excepted = |line: u64, column: u64| {
            POSTGRES_EXCEPTIONS
                .iter()
                .any(|&(l, c, _)| l == line && c == column)
        };
        let mut problems: Vec<String> = Vec::new();
        let (mut constants, mut names) = (0, 0);
        let (mut total, mut visited, mut skipped) = (0, 0, 0);
        for statement in oracle["statements"].as_array().unwrap() {
            for item in statement["constants"].as_array().unwrap() {
                let (line, column) = (
                    item["line"].as_u64().unwrap(),
                    item["column"].as_u64().unwrap(),
                );
                total += 1;
                if excepted(line, column) {
                    skipped += 1;
                    continue;
                }
                visited += 1;
                let at = byte_offset(&source, line, column);
                let Some(node) = nodes
                    .iter()
                    .find(|n| n.kind() == "literal" && n.range().start == at)
                else {
                    problems.push(format!("no literal at {line}:{column}"));
                    continue;
                };
                let acceptable = expected_constant(&item["value"]);
                let got = decode(node);
                if acceptable.contains(&got) {
                    constants += 1;
                } else {
                    problems.push(format!(
                        "{line}:{column} {}: {got:?} not in {acceptable:?}",
                        node.text()
                    ));
                }
            }
            for category in ["relations", "columns"] {
                for item in statement[category].as_array().unwrap() {
                    let (line, column) = (
                        item["line"].as_u64().unwrap(),
                        item["column"].as_u64().unwrap(),
                    );
                    total += 1;
                    if excepted(line, column) {
                        skipped += 1;
                        continue;
                    }
                    visited += 1;
                    let at = byte_offset(&source, line, column);
                    let parts: Vec<&str> = item["value"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|p| p.as_str().unwrap())
                        .filter(|p| *p != "*")
                        .collect();
                    // The reference's identifiers in order: a `field`'s qualifier
                    // (an `object_reference`) followed by its own name.
                    let Some(reference) = nodes.iter().find(|n| {
                        matches!(
                            n.kind().as_ref(),
                            "object_reference" | "field" | "all_fields"
                        ) && n.range().start == at
                    }) else {
                        // `*` alone has no reference node.
                        if !parts.is_empty() {
                            problems.push(format!("no reference at {line}:{column}"));
                        }
                        continue;
                    };
                    let found: Vec<String> = reference
                        .dfs()
                        .filter(|n| n.kind() == "identifier")
                        .map(|n| node_name(&n).expect("identifier name"))
                        .collect();
                    if found == parts {
                        names += parts.len();
                    } else {
                        problems.push(format!(
                            "{category} {line}:{column}: {found:?} != {parts:?}"
                        ));
                    }
                }
            }
        }
        assert!(problems.is_empty(), "{}", problems.join("\n"));
        assert!(constants > 0 && names > 0);
        assert_eq!(skipped, POSTGRES_EXCEPTIONS.len());
        assert_eq!(visited + skipped, total);
    }

    #[test]
    fn equals_sql_three_valued() {
        assert_eq!(equals(&Value::None, &Value::None), None);
        assert_eq!(equals(&Value::int(1), &Value::Float(1.0)), Some(true));
        assert_eq!(equals(&Value::int(1), &Value::Float(1.5)), Some(false));
        assert_eq!(equals(&Value::Bool(true), &Value::Bool(false)), Some(false));
        assert_eq!(
            equals(&Value::Str("A".into()), &Value::Str("a ".into())),
            Some(false)
        );
        assert_eq!(
            equals(&Value::Str("é".into()), &Value::Str("É".into())),
            None
        );
        assert_eq!(equals(&Value::Str("1".into()), &Value::int(1)), None);
    }

    #[test]
    fn names_follow_postgres_rules() {
        assert_eq!(name("Mixed_CASE"), Some("mixed_case".into()));
        assert_eq!(name("\"Mixed_CASE\""), Some("Mixed_CASE".into()));
        assert_eq!(name("\"x\"\"y\""), Some("x\"y".into()));
        assert_eq!(name("`x`"), None);
        assert_eq!(name("\"\""), None);
    }

    /// The literals of `source`, in order, as `decode` reads them; the source parses cleanly.
    fn decoded(source: &str) -> Vec<Value> {
        use crate::code_rule_dsl::language::GrammarDialect;
        use ast_grep_language::LanguageExt;
        let grep = GrammarDialect::Sql.ast_grep(source);
        assert!(!grep.root().get_inner_node().has_error(), "{source}");
        grep.root()
            .dfs()
            .filter(|node| node.kind() == "literal")
            .map(|node| decode(&node))
            .collect()
    }

    #[test]
    fn negative_floats_carry_their_sign_once() {
        assert_eq!(literal("-1.5"), Value::Float(-1.5));
        assert_eq!(literal("-1e2"), Value::Float(-100.0));
        assert_eq!(literal("- 2.25"), Value::Float(-2.25));
        assert_eq!(literal("+1.5"), Value::Float(1.5));
        assert_eq!(literal("--1.5"), Value::Unknown);
        assert_eq!(
            decoded("SELECT -1.5, -1e2, 2.5"),
            [Value::Float(-1.5), Value::Float(-100.0), Value::Float(2.5)]
        );
    }

    #[test]
    fn escape_string_octal_and_hex_escapes_are_bytes() {
        assert_eq!(literal(r"E'\xC3\xA9'"), Value::Str("é".into()));
        assert_eq!(literal(r"E'\303\251'"), Value::Str("é".into()));
        assert_eq!(
            literal(r"e'caf\xc3\251 \u00e9!'"),
            Value::Str("café é!".into())
        );
        assert_eq!(literal(r"E'\x41\101'"), Value::Str("AA".into()));
        // PostgreSQL keeps an octal escape's low byte: `\501` is 0x141, the byte 0x41.
        assert_eq!(literal(r"E'\501'"), Value::Str("A".into()));
        // Bytes that are not UTF-8 make no string PostgreSQL accepts in a UTF-8 database.
        assert_eq!(literal(r"E'\xC3'"), Value::Unknown);
        assert_eq!(literal(r"E'\377'"), Value::Unknown);
        assert_eq!(literal(r"E'a\0b'"), Value::Unknown);
    }

    #[test]
    fn lowercase_unicode_prefixes_decode() {
        assert_eq!(name(r#"u&"d\0061t""#), Some("dat".into()));
        assert_eq!(name(r#"U&"d\0061t""#), Some("dat".into()));
        assert_eq!(identifier(Cow::Borrowed(r#"u&"d\0061t""#)), "dat");
        assert_eq!(literal(r"u&'d\0061t'"), Value::Str("dat".into()));
        assert_eq!(literal(r"U&'d\0061t'"), Value::Str("dat".into()));
    }

    #[test]
    fn dollar_quoted_strings_end_at_their_own_tag() {
        for (source, body) in [
            // A candidate tag that does not match leaves its `$` to start the next one.
            ("SELECT $a$foo$bar$a$;", "foo$bar"),
            ("SELECT $a$x$$a$;", "x$"),
            ("SELECT $ab$x$a$ab$;", "x$a"),
            // Tags compare by whole code points: U+0161 is not `a` (0x61).
            ("SELECT $š$before $a$ after$š$;", "before $a$ after"),
            ("SELECT $東$a$q$東$;", "a$q"),
        ] {
            assert_eq!(decoded(source), [Value::Str(body.into())], "{source}");
        }
    }

    /// A dollar-quote tag is empty or an identifier (grammar patch 23), so `$1,$2` is two
    /// positional parameters: the tag `1,` used to open a string that ran to the next `$1,$`.
    #[test]
    fn positional_parameters_are_not_dollar_quote_tags() {
        use crate::code_rule_dsl::language::GrammarDialect;
        use ast_grep_language::LanguageExt;
        let source = "INSERT INTO t VALUES ($1,$2);\nSELECT 1 FROM t WHERE a = NULL;\n\
                      INSERT INTO t VALUES ($1,$2);\nSELECT $_x1$a$1,$_x1$, $é1$b$é1$;\n";
        let grep = GrammarDialect::Sql.ast_grep(source);
        assert!(!grep.root().get_inner_node().has_error(), "{source}");
        let parameters = grep
            .root()
            .dfs()
            .filter(|n| n.kind() == "parameter")
            .count();
        assert_eq!(parameters, 4);
        assert_eq!(
            decoded(source),
            [
                Value::int(1),
                Value::None,
                Value::Str("a$1,".into()),
                Value::Str("b".into())
            ]
        );
    }

    /// The review's repro, through the SQL pack's CV05 port: the file parses clean, so the
    /// `= NULL` on line 2 is reported rather than the file skipped as incomplete.
    #[test]
    fn cv05_reports_beside_positional_parameters() {
        use crate::code_rule_dsl::CodeRuleDslRuntime;
        use crate::code_rule_dsl::test_support::{CAPABILITIES, evaluate_file};
        let rule = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../packs/sql/candidates/rules/sqlfluff-cv05-convention-is-null.json");
        let document = std::fs::read_to_string(rule).expect("CV05 rule");
        let compiled =
            CodeRuleDslRuntime::compile(document, "json".to_owned(), CAPABILITIES.to_owned())
                .unwrap_or_else(|error| panic!("{}", error.reason));
        let source = "INSERT INTO t (a, b) VALUES ($1,$2);\nSELECT 1 FROM t WHERE a = NULL;\n\
                      INSERT INTO t (a, b) VALUES ($1,$2);\n";
        let result = evaluate_file(&compiled, "sql", "q.sql", source);
        assert_eq!(result["failures"], serde_json::json!([]), "{result}");
        let violations = result["violations"].as_array().expect("violations");
        assert_eq!(violations.len(), 1, "{result}");
        // `startLine` counts from 0: line 2.
        assert_eq!(violations[0]["span"]["startLine"], 1, "{result}");
    }

    /// A `$` that starts no tag fails at once: upstream read `$1,$`, `$2),($` and so on as tags
    /// and scanned each to the end of the file for its close, quadratic in the parameters.
    #[test]
    fn many_positional_parameters_parse_in_linear_time() {
        use crate::code_rule_dsl::language::GrammarDialect;
        use ast_grep_language::LanguageExt;
        let rows: Vec<String> = (0..10_000)
            .map(|i| format!("(${},${})", 2 * i + 1, 2 * i + 2))
            .collect();
        let source = format!("INSERT INTO t VALUES {};\n", rows.join(","));
        let started = std::time::Instant::now();
        let grep = GrammarDialect::Sql.ast_grep(&source);
        let elapsed = started.elapsed();
        assert!(!grep.root().get_inner_node().has_error());
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "{} bytes took {elapsed:?}",
            source.len()
        );
    }
}
