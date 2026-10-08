//! `printfFormat`: PHP's printf-family formats (`sprintf`, `printf`, `vsprintf`, `vprintf`,
//! `fprintf`, `vfprintf`), read as PHP 8.5.11's `php_formatted_print` reads them when every value
//! it asks for is passed; the oracle is that function and PHPStan 2.2.16's `PrintfFormatParser`
//! (`fixtures/php/literal/generate_printf.php`).
//!
//! The tree is a `format` root holding `literal` and `placeholder` parts in order. A placeholder
//! is `%[argnum$][flags][width][.precision][l]conversion`: it holds, when present, its `argnum`,
//! `flags` (`-`, `+`, space, `0` and `'` with its padding byte), `width`, `precision` (a `star`
//! inside each of the last two for `*`, `*N$`), `length` (`l`, which PHP skips) and `conversion`.
//! A placeholder's value is the index (from 0, after the format) of the value it consumes; a
//! star's, of the value its width or precision reads. `%%` is literal text, but a `%` conversion
//! after a modifier (`%5%`) is a placeholder that consumes a value, as PHP's is. A format PHP
//! refuses with a `ValueError` is a `format` root with one `error` node, valued by PHP's message.
//! Values are PHP's: strings are bytes, numbers integers.

use super::{Grammar, PNode};
use crate::code_rule_dsl::value::Value;

pub(crate) const KINDS: &[&str] = &[
    "format",
    "literal",
    "placeholder",
    "argnum",
    "flags",
    "width",
    "precision",
    "star",
    "length",
    "conversion",
    "error",
];

const G: Grammar = Grammar::Printf;

/// PHP's `INT_MAX`: an argument number, width or precision must be below it.
const INT_MAX: u64 = 2_147_483_647;

/// The conversions `php_formatted_print` knows (PHP 8 added `h` and `H`).
const CONVERSIONS: &[u8] = b"sdueEfFgGhHcoxXb%";

/// A `ValueError` PHP raises: its message and the bytes it is about.
struct Error {
    message: Vec<u8>,
    start: usize,
    end: usize,
}

impl Error {
    fn new(message: &str, start: usize, end: usize) -> Self {
        Self {
            message: message.as_bytes().to_vec(),
            start,
            end,
        }
    }
}

/// Parses `text` as `php_formatted_print` reads it with every value present.
pub(crate) fn parse(text: &str) -> PNode {
    let mut root = PNode::new(G, "format", 0, text.len());
    match parts(text) {
        Ok(parts) => root.children = parts,
        Err(error) => {
            let mut node = PNode::new(G, "error", floor(text, error.start), ceil(text, error.end));
            node.error = true;
            node.value = Some(Value::PhpStr(error.message));
            root.children = vec![node];
        }
    }
    root
}

/// The char boundary at or before `at` (PHP reads bytes; the tree's ranges are characters).
fn floor(text: &str, mut at: usize) -> usize {
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// The char boundary at or after `at`.
fn ceil(text: &str, mut at: usize) -> usize {
    while !text.is_char_boundary(at) {
        at += 1;
    }
    at
}

fn parts(text: &str) -> Result<Vec<PNode>, Error> {
    let bytes = text.as_bytes();
    let mut parts = Vec::new();
    // The literal being accumulated: its start and its bytes.
    let mut literal: Option<(usize, Vec<u8>)> = None;
    let flush = |literal: &mut Option<(usize, Vec<u8>)>, end: usize, parts: &mut Vec<PNode>| {
        if let Some((start, text)) = literal.take() {
            parts.push(PNode::new(G, "literal", start, end).with_value(Value::PhpStr(text)));
        }
    };
    let mut next = 0;
    let mut index = 0;
    while index < bytes.len() {
        let Some(percent) = bytes[index..].iter().position(|byte| *byte == b'%') else {
            literal
                .get_or_insert_with(|| (index, Vec::new()))
                .1
                .extend_from_slice(&bytes[index..]);
            index = bytes.len();
            break;
        };
        let percent = index + percent;
        if percent > index {
            literal
                .get_or_insert_with(|| (index, Vec::new()))
                .1
                .extend_from_slice(&bytes[index..percent]);
        }
        if bytes.get(percent + 1) == Some(&b'%') {
            literal
                .get_or_insert_with(|| (percent, Vec::new()))
                .1
                .push(b'%');
            index = percent + 2;
            continue;
        }
        flush(&mut literal, percent, &mut parts);
        let (placeholder, end) = placeholder(bytes, percent, &mut next)?;
        parts.push(placeholder);
        index = end;
    }
    flush(&mut literal, index, &mut parts);
    Ok(parts)
}

/// The specifier after the `%` at `percent`: the placeholder and the index after it. `next` is
/// PHP's `currarg`, the value an unnumbered placeholder or star consumes next.
fn placeholder(bytes: &[u8], percent: usize, next: &mut usize) -> Result<(PNode, usize), Error> {
    let mut at = percent + 1;
    let mut children = Vec::new();
    let mut argnum = None;
    // A letter right after `%` is the conversion (or `l`): no argument number or modifiers.
    if !bytes.get(at).is_some_and(u8::is_ascii_alphabetic) {
        if let Some((index, node)) = argument_number(bytes, &mut at)? {
            argnum = Some(index);
            children.push(node);
        }
        let flags = at;
        loop {
            match bytes.get(at) {
                Some(b' ' | b'0' | b'-' | b'+') => at += 1,
                Some(b'\'') => {
                    if bytes.len() - at > 1 {
                        at += 2;
                    } else {
                        return Err(Error::new("Missing padding character", at, at + 1));
                    }
                }
                _ => break,
            }
        }
        if at > flags {
            children.push(
                PNode::new(G, "flags", flags, at)
                    .with_value(Value::PhpStr(bytes[flags..at].to_vec())),
            );
        }
        if bytes.get(at) == Some(&b'*') {
            let start = at;
            children.push(star(bytes, &mut at, "width", start, next)?);
        } else if bytes.get(at).is_some_and(u8::is_ascii_digit) {
            let start = at;
            let width = number(bytes, &mut at)
                .ok_or_else(|| Error::new("Width must be between 0 and 2147483647", start, at))?;
            children.push(PNode::new(G, "width", start, at).with_value(Value::int(width as i64)));
        }
        if bytes.get(at) == Some(&b'.') {
            let dot = at;
            at += 1;
            if bytes.get(at) == Some(&b'*') {
                children.push(star(bytes, &mut at, "precision", dot, next)?);
            } else if bytes.get(at).is_some_and(u8::is_ascii_digit) {
                let start = at;
                let precision = number(bytes, &mut at).ok_or_else(|| {
                    Error::new("Precision must be between 0 and 2147483647", start, at)
                })?;
                children.push(
                    PNode::new(G, "precision", dot, at).with_value(Value::int(precision as i64)),
                );
            } else {
                children.push(PNode::new(G, "precision", dot, at));
            }
        }
    }
    if bytes.get(at) == Some(&b'l') {
        children.push(PNode::new(G, "length", at, at + 1));
        at += 1;
    }
    let index = argnum.unwrap_or_else(|| take(next));
    let Some(&conversion) = bytes.get(at) else {
        return Err(Error::new(
            "Missing format specifier at end of string",
            at,
            at,
        ));
    };
    if !CONVERSIONS.contains(&conversion) {
        let mut message = b"Unknown format specifier \"".to_vec();
        // PHP formats the message as a C string: a NUL specifier ends it.
        if conversion != 0 {
            message.push(conversion);
            message.push(b'"');
        }
        return Err(Error {
            message,
            start: at,
            end: at + 1,
        });
    }
    children
        .push(PNode::new(G, "conversion", at, at + 1).with_value(Value::PhpStr(vec![conversion])));
    at += 1;
    let mut node = PNode::new(G, "placeholder", percent, at).with_value(Value::int(index as i64));
    node.children = children;
    Ok((node, at))
}

/// PHP's `currarg++`.
fn take(next: &mut usize) -> usize {
    *next += 1;
    *next - 1
}

/// `php_sprintf_get_argnum`: digits followed by `$` number a value from 1 (an `argnum` node,
/// valued by its index from 0); `None` without a `$`.
fn argument_number(bytes: &[u8], at: &mut usize) -> Result<Option<(usize, PNode)>, Error> {
    let start = *at;
    let digits = bytes[start..]
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if bytes.get(start + digits) != Some(&b'$') {
        return Ok(None);
    }
    let mut end = start;
    let number = number(bytes, &mut end).unwrap_or(0);
    if number == 0 {
        return Err(Error::new(
            "Argument number specifier must be greater than zero and less than 2147483647",
            start,
            start + digits + 1,
        ));
    }
    *at = start + digits + 1;
    let index = number as usize - 1;
    Ok(Some((
        index,
        PNode::new(G, "argnum", start, *at).with_value(Value::int(index as i64)),
    )))
}

/// `php_sprintf_getnumber` over the digits at `at`: their value, or `None` at `INT_MAX` or past.
fn number(bytes: &[u8], at: &mut usize) -> Option<u64> {
    let mut value: u64 = 0;
    let mut overflow = false;
    while let Some(digit) = bytes.get(*at).filter(|byte| byte.is_ascii_digit()) {
        value = value * 10 + u64::from(digit - b'0');
        overflow |= value >= INT_MAX;
        if overflow {
            value = INT_MAX;
        }
        *at += 1;
    }
    (!overflow).then_some(value)
}

/// A `*` width or precision (`kind`) from `start` (the `.` of a precision): a `star` valued by the
/// value it reads, numbered (`*N$`) or the next.
fn star(
    bytes: &[u8],
    at: &mut usize,
    kind: &str,
    start: usize,
    next: &mut usize,
) -> Result<PNode, Error> {
    let asterisk = *at;
    *at += 1;
    let index = match argument_number(bytes, at)? {
        Some((index, _)) => index,
        None => take(next),
    };
    let star = PNode::new(G, "star", asterisk, *at).with_value(Value::int(index as i64));
    let mut node = PNode::new(G, kind, start, *at);
    node.children.push(star);
    Ok(node)
}

#[cfg(test)]
mod tests;
