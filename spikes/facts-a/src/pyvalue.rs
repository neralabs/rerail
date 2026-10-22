//! Python constant values of literal nodes (native `value/python.rs`), restricted to what the
//! batch-6 `value` tests read: string literals and their implicit concatenation, the keyword
//! constants and numbers as non-string values. Operators, calls and containers fold through
//! builtins whose resolution needs scope facts outside schema v0: they are refused, never guessed.
use crate::facts::{self, Facts};

pub const MAX_STRING_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub enum PyValue {
    Str(String),
    Bytes(Vec<u8>),
    /// A known value that is neither str nor bytes (number, bool, None, Ellipsis).
    Other,
    Unknown,
}

pub const EXPRESSION_KINDS: &[&str] = &[
    "as_pattern", "attribute", "await", "binary_operator", "boolean_operator", "call",
    "comparison_operator", "concatenated_string", "conditional_expression", "dictionary",
    "dictionary_comprehension", "ellipsis", "expression_list", "false", "float",
    "generator_expression", "identifier", "integer", "lambda", "list", "list_comprehension",
    "list_splat", "named_expression", "none", "not_operator", "parenthesized_expression", "set",
    "set_comprehension", "string", "subscript", "true", "tuple", "unary_operator", "yield",
];

/// The node's value; `Err` where folding needs facts the spike does not have.
pub fn decode(f: &Facts, n: u32) -> Result<PyValue, String> {
    let kind = facts::name(f.kind[n as usize]);
    let kind: &str = &kind;
    Ok(match kind {
        "integer" | "float" | "true" | "false" | "none" | "ellipsis" => {
            if kind == "integer" || kind == "float" {
                // A number literal Python refuses is Unknown natively; refuse to guess.
                if !number_ok(f.text(n)) {
                    return Err(format!("value of number literal `{}`", f.text(n)));
                }
            }
            PyValue::Other
        }
        "string" => string(f, n),
        "concatenated_string" => concatenated(f, n),
        "parenthesized_expression" => {
            let named: Vec<u32> = f
                .children(n)
                .iter()
                .copied()
                .filter(|&c| f.named(c) && &*facts::name(f.kind[c as usize]) != "comment")
                .collect();
            match named.as_slice() {
                [inner] => decode(f, *inner)?,
                _ => PyValue::Unknown,
            }
        }
        "unary_operator" | "not_operator" | "binary_operator" | "boolean_operator"
        | "comparison_operator" | "conditional_expression" | "call" | "tuple" | "list" | "set"
        | "dictionary" => return Err(format!("value folding of `{kind}` is outside schema v0")),
        // `follow` is off in every batch-6 value test: a name has no value.
        _ => PyValue::Unknown,
    })
}

fn number_ok(t: &str) -> bool {
    let t = t.replace('_', "");
    let l = t.to_ascii_lowercase();
    l.parse::<f64>().is_ok()
        || l.strip_prefix("0x").is_some_and(|h| u128::from_str_radix(h, 16).is_ok())
        || l.strip_prefix("0o").is_some_and(|h| u128::from_str_radix(h, 8).is_ok())
        || l.strip_prefix("0b").is_some_and(|h| u128::from_str_radix(h, 2).is_ok())
        || l.trim_end_matches(['j', 'l']).parse::<f64>().is_ok()
}

fn has_interpolation(f: &Facts, n: u32) -> bool {
    f.children(n).iter().any(|&c| &*facts::name(f.kind[c as usize]) == "interpolation")
}

fn string(f: &Facts, n: u32) -> PyValue {
    if has_interpolation(f, n) {
        return PyValue::Unknown;
    }
    match piece(f.text(n)) {
        Some(Piece::Str(s)) => PyValue::Str(s),
        Some(Piece::Bytes(b)) => PyValue::Bytes(b),
        None => PyValue::Unknown,
    }
}

fn concatenated(f: &Facts, n: u32) -> PyValue {
    let mut joined: Option<Piece> = None;
    for &part in f.children(n) {
        if &*facts::name(f.kind[part as usize]) != "string" {
            continue;
        }
        if has_interpolation(f, part) {
            return PyValue::Unknown;
        }
        let Some(next) = piece(f.text(part)) else { return PyValue::Unknown };
        joined = Some(match (joined, next) {
            (None, next) => next,
            (Some(Piece::Str(mut l)), Piece::Str(r)) => {
                l.push_str(&r);
                Piece::Str(l)
            }
            (Some(Piece::Bytes(mut l)), Piece::Bytes(r)) => {
                l.extend(r);
                Piece::Bytes(l)
            }
            _ => return PyValue::Unknown,
        });
        let size = match &joined {
            Some(Piece::Str(v)) => v.len(),
            Some(Piece::Bytes(v)) => v.len(),
            None => 0,
        };
        if size > MAX_STRING_BYTES {
            return PyValue::Unknown;
        }
    }
    match joined {
        Some(Piece::Str(v)) => PyValue::Str(v),
        Some(Piece::Bytes(v)) => PyValue::Bytes(v),
        None => PyValue::Unknown,
    }
}

#[derive(Clone, Copy)]
pub struct Prefix {
    pub raw: bool,
    pub bytes: bool,
    pub format: bool,
    pub template: bool,
}

pub fn prefix(letters: &str) -> Option<Prefix> {
    let lower = letters.to_ascii_lowercase();
    matches!(lower.as_str(), "" | "r" | "u" | "b" | "br" | "rb" | "f" | "fr" | "rf" | "t" | "tr" | "rt").then(|| Prefix {
        raw: lower.contains('r'),
        bytes: lower.contains('b'),
        format: lower.contains('f'),
        template: lower.contains('t'),
    })
}

enum Piece {
    Str(String),
    Bytes(Vec<u8>),
}

fn piece(text: &str) -> Option<Piece> {
    let quote_at = text.find(['\'', '"'])?;
    let prefix = prefix(&text[..quote_at])?;
    if prefix.template {
        return None;
    }
    let rest = &text[quote_at..];
    let quote = if rest.starts_with("'''") || rest.starts_with("\"\"\"") { &rest[..3] } else { &rest[..1] };
    if rest.len() < 2 * quote.len() || !rest.ends_with(quote) {
        return None;
    }
    decode_body(&rest[quote.len()..rest.len() - quote.len()], &prefix)
}

fn decode_body(body: &str, prefix: &Prefix) -> Option<Piece> {
    let mut body = body.replace("\r\n", "\n").replace('\r', "\n");
    if prefix.format || prefix.template {
        body = body.replace("{{", "{").replace("}}", "}");
    }
    if prefix.bytes {
        if !body.is_ascii() {
            return None;
        }
        let value = if prefix.raw {
            body.into_bytes()
        } else {
            decode_escapes(&body, true)?.into_iter().map(|u| u8::try_from(u).ok()).collect::<Option<Vec<_>>>()?
        };
        return (value.len() <= MAX_STRING_BYTES).then_some(Piece::Bytes(value));
    }
    let value = if prefix.raw {
        body
    } else {
        decode_escapes(&body, false)?.into_iter().map(char::from_u32).collect::<Option<String>>()?
    };
    (value.len() <= MAX_STRING_BYTES).then_some(Piece::Str(value))
}

/// Python's escapes as code points. `\N{name}` needs the Unicode name table: refused upstream by
/// returning `None` only when the name is unknown natively; the spike has no table, so it treats
/// every `\N{...}` as unknown (`None`) and flags it (see `NAMED_ESCAPES`).
fn decode_escapes(body: &str, bytes: bool) -> Option<Vec<u32>> {
    let mut out = Vec::with_capacity(body.len());
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(u32::from(c));
            continue;
        }
        let escape = chars.next()?;
        let simple = match escape {
            '\n' => continue,
            '\\' => Some(u32::from('\\')),
            '\'' => Some(u32::from('\'')),
            '"' => Some(u32::from('"')),
            'a' => Some(7),
            'b' => Some(8),
            'f' => Some(12),
            'n' => Some(10),
            'r' => Some(13),
            't' => Some(9),
            'v' => Some(11),
            _ => None,
        };
        if let Some(code) = simple {
            out.push(code);
            continue;
        }
        let mut hex = |count: usize| {
            let digits: String = chars.by_ref().take(count).collect();
            (digits.len() == count && digits.chars().all(|c| c.is_ascii_hexdigit()))
                .then(|| u32::from_str_radix(&digits, 16).ok())
                .flatten()
        };
        match escape {
            '0'..='7' => {
                let mut code = escape.to_digit(8)?;
                for _ in 0..2 {
                    match chars.peek().and_then(|c| c.to_digit(8)) {
                        Some(d) => {
                            code = code * 8 + d;
                            chars.next();
                        }
                        None => break,
                    }
                }
                out.push(if bytes { code & 0xff } else { code });
            }
            'x' => out.push(hex(2)?),
            'u' if !bytes => out.push(char::from_u32(hex(4)?).map(u32::from)?),
            'U' if !bytes => out.push(char::from_u32(hex(8)?).map(u32::from)?),
            'N' if !bytes => {
                NAMED_ESCAPES.with(|n| n.set(n.get() + 1));
                return None;
            }
            other => {
                out.push(u32::from('\\'));
                out.push(u32::from(other));
            }
        }
    }
    Some(out)
}

thread_local! {
    /// `\N{...}` escapes met (decoded as unknown; the native engine knows the names). A non-zero
    /// count makes the evaluation refuse.
    pub static NAMED_ESCAPES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}
