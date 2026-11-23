//! A Python string literal's decoded text with the source span of every decoded character (`offsets::Decoded`),
//! and a text grammar's tree of it (`parse_string`).

use super::offsets::{Decode, Decoded, Simple, push};
use crate::code_rule_dsl::value::MAX_STRING_BYTES;
use crate::code_rule_dsl::value::names;
use crate::code_rule_dsl::value::python::{Prefix, prefix};
use ast_grep_core::{Doc, Node};

/// Decodes a Python `string` or `concatenated_string` node as `str`, recording spans.
pub fn decode_python<D: Doc>(node: &Node<'_, D>) -> Decode {
    let parts: Vec<Node<'_, D>> = match node.kind().as_ref() {
        "string" => vec![node.clone()],
        "concatenated_string" => node
            .children()
            .filter(|child| child.kind() == "string")
            .collect(),
        _ => return Decode::NotString,
    };
    let mut decoded = Decoded {
        text: String::new(),
        spans: Vec::new(),
        host: node.range(),
        simple: None,
    };
    let single = parts.len() == 1;
    for part in &parts {
        if part
            .children()
            .any(|child| matches!(child.kind().as_ref(), "interpolation"))
        {
            return Decode::Unknown;
        }
        let text = part.text();
        let base = part.range().start;
        let Some(quote_at) = text.find(['\'', '"']) else {
            return Decode::Unknown;
        };
        let Some(prefix) = prefix(&text[..quote_at]) else {
            return Decode::Unknown;
        };
        if prefix.template || prefix.bytes {
            return Decode::Unknown;
        }
        let rest = &text[quote_at..];
        let quote = if rest.starts_with("'''") || rest.starts_with("\"\"\"") {
            3
        } else {
            1
        };
        if rest.len() < 2 * quote || !rest.ends_with(&rest[..quote]) {
            return Decode::Unknown;
        }
        let body = &rest[quote..rest.len() - quote];
        let body_start = base + quote_at + quote;
        let before = decoded.text.len();
        if !decode_body(body, body_start, &prefix, &mut decoded) {
            return Decode::Unknown;
        }
        if decoded.text.len() > MAX_STRING_BYTES {
            return Decode::Unknown;
        }
        if single && !prefix.format && decoded.text[before..] == *body {
            decoded.simple = Some(Simple {
                content: body_start,
                triple: quote == 3,
            });
        }
    }
    Decode::Known(decoded)
}

/// Decodes one literal's body into `decoded`: universal newlines, an f-string's doubled braces,
/// escapes unless raw. False for an escape Python refuses or a name `\N{...}` does not know.
pub(crate) fn decode_body(body: &str, base: usize, prefix: &Prefix, decoded: &mut Decoded) -> bool {
    let bytes = body.as_bytes();
    let mut index = 0;
    while index < body.len() {
        let c = body[index..].chars().next().expect("in bounds");
        let at = base + index;
        match c {
            '\r' => {
                let width = if bytes.get(index + 1) == Some(&b'\n') {
                    2
                } else {
                    1
                };
                push(decoded, '\n', at, at + width);
                index += width;
            }
            '{' | '}' if prefix.format && bytes.get(index + 1) == Some(&(c as u8)) => {
                push(decoded, c, at, at + 2);
                index += 2;
            }
            '\\' if !prefix.raw => {
                let Some(next) = escape(body, index, at, decoded) else {
                    return false;
                };
                index = next;
            }
            _ => {
                push(decoded, c, at, at + c.len_utf8());
                index += c.len_utf8();
            }
        }
    }
    true
}

/// Decodes the escape at `index` (a backslash), returning the index after it.
fn escape(body: &str, index: usize, at: usize, decoded: &mut Decoded) -> Option<usize> {
    let rest = &body[index + 1..];
    let escape = rest.chars().next()?;
    let after = index + 1 + escape.len_utf8();
    let whole = |end: usize| at..at + (end - index);
    let simple = match escape {
        '\n' => return Some(after),
        '\r' => {
            return Some(if rest.as_bytes().get(1) == Some(&b'\n') {
                after + 1
            } else {
                after
            });
        }
        '\\' => Some('\\'),
        '\'' => Some('\''),
        '"' => Some('"'),
        'a' => Some('\u{7}'),
        'b' => Some('\u{8}'),
        'f' => Some('\u{c}'),
        'n' => Some('\n'),
        'r' => Some('\r'),
        't' => Some('\t'),
        'v' => Some('\u{b}'),
        _ => None,
    };
    if let Some(c) = simple {
        let range = whole(after);
        push(decoded, c, range.start, range.end);
        return Some(after);
    }
    let hex = |count: usize| -> Option<(u32, usize)> {
        let digits = rest.get(1..1 + count)?;
        (digits.len() == count && digits.chars().all(|c| c.is_ascii_hexdigit()))
            .then(|| u32::from_str_radix(digits, 16).ok())
            .flatten()
            .map(|code| (code, after + count))
    };
    let (code, end) = match escape {
        '0'..='7' => {
            let mut code = escape.to_digit(8)?;
            let mut end = after;
            for digit in rest[1..].chars().take(2) {
                match digit.to_digit(8) {
                    Some(digit) => {
                        code = code * 8 + digit;
                        end += 1;
                    }
                    None => break,
                }
            }
            (code, end)
        }
        'x' => hex(2)?,
        'u' => hex(4)?,
        'U' => hex(8)?,
        'N' => {
            let name = rest.strip_prefix("N{")?;
            let close = name.find('}')?;
            let c = names::lookup(&name[..close])?;
            (u32::from(c), after + 1 + close + 1)
        }
        other => {
            push(decoded, '\\', at, at + 1);
            push(decoded, other, at + 1, at + 1 + other.len_utf8());
            return Some(after);
        }
    };
    let c = char::from_u32(code)?;
    let range = whole(end);
    push(decoded, c, range.start, range.end);
    Some(end)
}

/// Parses a Python string node with a text grammar (not `annotation`, which the scope table
/// parses). Spends one step of `spend` per tree node; `None` when the budget ran out.
pub fn parse_string<D: ast_grep_core::Doc>(
    node: &ast_grep_core::Node<'_, D>,
    grammar: super::Grammar,
    spend: &mut dyn FnMut(usize) -> bool,
) -> Option<super::Parsed> {
    let decoded = match decode_python(node) {
        super::offsets::Decode::Known(decoded) => decoded,
        super::offsets::Decode::Unknown => return Some(super::Parsed::Unknown),
        super::offsets::Decode::NotString => return Some(super::Parsed::NotString),
    };
    if !spend(decoded.text.len() + 1) {
        return None;
    }
    let root = grammar.parse(&decoded.text);
    if !spend(root.size()) {
        return None;
    }
    let text = decoded.text.clone();
    Some(super::Parsed::Tree(std::sync::Arc::new(super::SubTree::new(
        grammar,
        text,
        0,
        root,
        super::HostMap::Spans(decoded),
    ))))
}
