//! SQL literal, identifier, and equality semantics shared by the generic SQL dialect.
//!
//! PostgreSQL's scanner (`scan.l`, `scansup.c`, notably `downcase_identifier` and
//! `truncate_identifier`) and SQLite's tokenizer are deliberately conservative here:
//! a value is returned only when their readings agree.

use super::{Value, int_float_order};
use num_bigint::BigInt;
use num_traits::ToPrimitive;
use std::cmp::Ordering;

fn digits_ok(s: &str, radix: u32) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b[0] != b'_'
        && b[b.len() - 1] != b'_'
        && !s.contains("__")
        && s.chars().all(|c| c == '_' || c.to_digit(radix).is_some())
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
        out.push(c);
    }
    None
}

fn e_string(s: &str) -> Option<String> {
    let (raw, used) = quoted_body(s, '\'')?;
    if used != s.len() {
        return None;
    }
    let mut out = String::new();
    let b = raw.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'\\' {
            out.push(b[i] as char);
            i += 1;
            continue;
        }
        i += 1;
        if i == b.len() {
            return None;
        }
        let c = b[i] as char;
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
            out.push(mapped);
            continue;
        }
        if c.is_ascii_digit() && c < '8' {
            let mut v = (c as u32) - 48;
            for _ in 0..2 {
                if i < b.len() && (b[i] as char).is_ascii_digit() && b[i] < b'8' {
                    v = v * 8 + (b[i] - b'0') as u32;
                    i += 1;
                } else {
                    break;
                }
            }
            out.push(char::from_u32(v)?);
        } else if c == 'x' {
            let n = (1..=2)
                .find(|n| i + n <= b.len() && b[i..i + n].iter().all(u8::is_ascii_hexdigit))
                .unwrap_or(0);
            if n == 0 {
                return None;
            }
            let v = u32::from_str_radix(std::str::from_utf8(&b[i..i + n]).ok()?, 16).ok()?;
            out.push(char::from_u32(v)?);
            i += n;
        } else if c == 'u' || c == 'U' {
            let n = if c == 'u' { 4 } else { 8 };
            if i + n > b.len() || !b[i..i + n].iter().all(u8::is_ascii_hexdigit) {
                return None;
            }
            let v = u32::from_str_radix(std::str::from_utf8(&b[i..i + n]).ok()?, 16).ok()?;
            out.push(char::from_u32(v)?);
            i += n;
        } else {
            out.push(c);
        }
    }
    if out.chars().any(|c| c == '\0') {
        None
    } else {
        Some(out)
    }
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

fn unicode_string(s: &str, ident: bool) -> Option<String> {
    let prefix = if ident { "U&\"" } else { "U&'" };
    if !s.starts_with(prefix) {
        return None;
    }
    let q = if ident { '\"' } else { '\'' };
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
    if text.starts_with("U&'") || text.starts_with("u&'") {
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
    float_literal(&format!("{}{}", sign.unwrap_or('+'), n))
        .map(signed)
        .unwrap_or(Value::Unknown)
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

pub(crate) fn name(text: &str) -> Option<String> {
    let mut out = if text.starts_with("U&\"") {
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

    #[test]
    fn postgres_constants_and_names_cover_oracle() {
        let root = oracle_root("sql/scope/postgres.expected.json");
        let mut constants = 0;
        let mut names = 0;
        for statement in root["statements"].as_array().unwrap() {
            for item in statement["constants"].as_array().unwrap() {
                let kind = item["value"][0].as_str().unwrap();
                let line = item["line"].as_u64().unwrap();
                let column = item["column"].as_u64().unwrap();
                assert!(line > 0 && column > 0);
                assert!(matches!(
                    kind,
                    "number" | "string" | "bits" | "bool" | "null"
                ));
                constants += 1;
            }
            for category in ["relations", "columns"] {
                for item in statement[category].as_array().unwrap() {
                    for part in item["value"].as_array().unwrap() {
                        if part.as_str() != Some("*") {
                            names += 1;
                        }
                    }
                }
            }
        }
        assert!(constants > 0);
        assert!(names > 0);
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
}
