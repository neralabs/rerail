//! `scanfFormat`: PHP's `sscanf`/`fscanf` formats, read as PHP 8.5.11's `ValidateFormat`
//! (`ext/standard/scanf.c`) reads them when no variable is passed (the call returns an array);
//! the oracle is that function (`fixtures/php/literal/generate_scanf.php`).
//!
//! The tree is a `format` root holding `literal` and `placeholder` parts in order. A placeholder
//! is `%[*|argnum$][width][l|L|h]conversion`: it holds, when present, its `suppress` (`*`, which
//! assigns nothing), `argnum`, `width`, `length` and `conversion` (`n d D i o x X u f e E g s c`,
//! or `[` with a `set` child for the bracketed set). A placeholder that assigns is valued by the
//! index (from 0) of the variable it assigns; an `argnum` by the index it names. PHP reads the
//! format as a C string: what follows a NUL byte is one `literal` it never reads. A format PHP
//! refuses with a `ValueError` is a `format` root with one `error` node valued by PHP's message.

use super::{Grammar, PNode};
use crate::policy_dsl::value::Value;
use std::collections::BTreeMap;

pub(crate) const KINDS: &[&str] = &[
    "format",
    "literal",
    "placeholder",
    "suppress",
    "argnum",
    "width",
    "length",
    "conversion",
    "set",
    "error",
];

const G: Grammar = Grammar::Scanf;

/// `SCAN_MAX_ARGS`: the highest `%n$` a format may name when no variable is passed.
const MAX_ARGS: i64 = 0xFF;

const CONVERSIONS: &[u8] = b"ndDioxXufeEgsc";

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

/// Parses `text` as `ValidateFormat` reads it with no variable passed.
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

fn floor(text: &str, mut at: usize) -> usize {
    at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

fn ceil(text: &str, mut at: usize) -> usize {
    at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at += 1;
    }
    at
}

/// The scan's state across placeholders: `ValidateFormat`'s `gotXpg`, `gotSequential`,
/// `objIndex`, `xpgSize`, and the placeholders that assign each index (`nassign`).
#[derive(Default)]
struct Scan {
    xpg: bool,
    sequential: bool,
    next: i64,
    xpg_size: i64,
    assigned: BTreeMap<i64, Vec<(usize, usize)>>,
}

fn parts(text: &str) -> Result<Vec<PNode>, Error> {
    let all = text.as_bytes();
    // PHP reads a C string: nothing past a NUL.
    let end = all.iter().position(|byte| *byte == 0).unwrap_or(all.len());
    let bytes = &all[..end];
    let mut parts = Vec::new();
    let mut literal: Option<(usize, Vec<u8>)> = None;
    let flush = |literal: &mut Option<(usize, Vec<u8>)>, end: usize, parts: &mut Vec<PNode>| {
        if let Some((start, text)) = literal.take() {
            parts.push(PNode::new(G, "literal", start, end).with_value(Value::PhpStr(text)));
        }
    };
    let mut scan = Scan::default();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            literal
                .get_or_insert_with(|| (index, Vec::new()))
                .1
                .push(bytes[index]);
            index += 1;
            continue;
        }
        if bytes.get(index + 1) == Some(&b'%') {
            literal
                .get_or_insert_with(|| (index, Vec::new()))
                .1
                .push(b'%');
            index += 2;
            continue;
        }
        flush(&mut literal, index, &mut parts);
        let (placeholder, next) = placeholder(bytes, index, &mut scan)?;
        parts.push(placeholder);
        index = next;
    }
    flush(&mut literal, index, &mut parts);
    if end < all.len() {
        parts.push(
            PNode::new(G, "literal", end, all.len()).with_value(Value::PhpStr(all[end..].to_vec())),
        );
    }
    // Every variable a `%n$` names is assigned once.
    for spans in scan.assigned.values() {
        if let Some((start, end)) = spans.get(1) {
            return Err(Error::new(
                "Variable is assigned by multiple \"%n$\" conversion specifiers",
                *start,
                *end,
            ));
        }
    }
    Ok(parts)
}

/// The specifier after the `%` at `percent`: the placeholder and the index after it.
fn placeholder(bytes: &[u8], percent: usize, scan: &mut Scan) -> Result<(PNode, usize), Error> {
    let mut at = percent + 1;
    let mut children = Vec::new();
    let mut suppress = false;
    let mut index = None;
    let digit = |at: usize| bytes.get(at).is_some_and(u8::is_ascii_digit);
    let digits_end = |at: usize| {
        at + bytes[at..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count()
    };
    if bytes.get(at) == Some(&b'*') {
        suppress = true;
        children.push(PNode::new(G, "suppress", at, at + 1));
        at += 1;
    } else if digit(at) && bytes.get(digits_end(at)) == Some(&b'$') {
        let end = digits_end(at);
        // `strtoul` saturates; its result is stored in an `int`.
        let value = bytes[at..end]
            .iter()
            .try_fold(0u64, |value, digit| {
                value
                    .checked_mul(10)
                    .and_then(|value| value.checked_add(u64::from(digit - b'0')))
            })
            .unwrap_or(u64::MAX);
        let value = i64::from(value as u32 as i32);
        scan.xpg = true;
        if scan.sequential {
            return Err(mixed(percent, end + 1));
        }
        if !(1..=MAX_ARGS).contains(&value) {
            return Err(Error::new(
                "\"%n$\" argument index out of range",
                at,
                end + 1,
            ));
        }
        scan.xpg_size = scan.xpg_size.max(value);
        scan.next = value - 1;
        children.push(PNode::new(G, "argnum", at, end + 1).with_value(Value::int(value - 1)));
        at = end + 1;
    } else {
        scan.sequential = true;
        if scan.xpg {
            return Err(mixed(percent, at));
        }
    }
    if digit(at) {
        let end = digits_end(at);
        let width = bytes[at..end].iter().fold(0u64, |value, digit| {
            value
                .saturating_mul(10)
                .saturating_add(u64::from(digit - b'0'))
        });
        children.push(
            PNode::new(G, "width", at, end)
                .with_value(Value::int(width.min(i64::MAX as u64) as i64)),
        );
        at = end;
    }
    if matches!(bytes.get(at), Some(b'l' | b'L' | b'h')) {
        children.push(PNode::new(G, "length", at, at + 1));
        at += 1;
    }
    match bytes.get(at) {
        Some(&c) if CONVERSIONS.contains(&c) => {
            children
                .push(PNode::new(G, "conversion", at, at + 1).with_value(Value::PhpStr(vec![c])));
            at += 1;
        }
        Some(b'[') => {
            let open = at;
            let bad = || Error::new("Unmatched [ in format string", open, bytes.len());
            // ValidateFormat's walk: a leading `^`, then a leading `]`, belong to the set.
            let mut ch = open;
            let step = |ch: &mut usize| {
                if *ch + 1 >= bytes.len() {
                    return Err(bad());
                }
                *ch += 1;
                Ok(())
            };
            step(&mut ch)?;
            if bytes[ch] == b'^' {
                step(&mut ch)?;
            }
            if bytes[ch] == b']' {
                step(&mut ch)?;
            }
            while bytes[ch] != b']' {
                step(&mut ch)?;
            }
            let mut conversion =
                PNode::new(G, "conversion", open, ch + 1).with_value(Value::PhpStr(b"[".to_vec()));
            conversion.children.push(
                PNode::new(G, "set", open + 1, ch)
                    .with_value(Value::PhpStr(bytes[open + 1..ch].to_vec())),
            );
            children.push(conversion);
            at = ch + 1;
        }
        other => {
            let mut message = b"Bad scan conversion character \"".to_vec();
            // PHP formats the message as a C string: a NUL (the format's end) ends it.
            if let Some(&c) = other {
                message.push(c);
                message.push(b'"');
            }
            return Err(Error {
                message,
                start: at,
                end: (at + 1).min(bytes.len()),
            });
        }
    }
    if !suppress {
        let assigned = scan.next;
        scan.assigned
            .entry(assigned)
            .or_default()
            .push((percent, at));
        scan.next += 1;
        index = Some(assigned);
    }
    let mut node = PNode::new(G, "placeholder", percent, at);
    node.value = index.map(Value::int);
    node.children = children;
    Ok((node, at))
}

fn mixed(start: usize, end: usize) -> Error {
    Error::new(
        "cannot mix \"%\" and \"%n$\" conversion specifiers",
        start,
        end,
    )
}

#[cfg(test)]
mod tests;
