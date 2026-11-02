//! Python identifiers as CPython reads them: the tokenizer NFKC-normalizes every identifier
//! (PEP 3131), so `𝒞` and `C` name one binding, attribute, keyword or module. Ruff's lexer does
//! the same. Names the Python family extracts to compare, resolve or qualify go through
//! `python_name`; violations, spans and `regex` keep the source text, and string contents (an
//! `__all__` entry, a `getattr` argument) stay raw, as CPython keeps them. ECMAScript names are
//! compared as espree's `Identifier.name` holds them, their `\uXXXX` and `\u{X}` escapes decoded
//! (`ecma_name`), code unit by code unit; PHP names as written (bytes).

use crate::code_rule_dsl::language::adapter::{Adapter, with_adapter};
use crate::code_rule_dsl::language::{Family, GrammarDialect};
use ast_grep_core::{Doc, Node};
use std::borrow::Cow;
use unicode_normalization::{IsNormalized, UnicodeNormalization, is_nfkc_quick};

/// `text` under NFKC; borrowed, without allocating, where it is already normal (any ASCII text).
pub(crate) fn nfkc(text: &str) -> Cow<'_, str> {
    if text.is_ascii() || is_nfkc_quick(text.chars()) == IsNormalized::Yes {
        Cow::Borrowed(text)
    } else {
        Cow::Owned(text.nfkc().collect())
    }
}

/// A Python identifier's text, NFKC-normalized; `text` itself where it is already normal.
pub(crate) fn python_name(text: Cow<'_, str>) -> Cow<'_, str> {
    match nfkc(&text) {
        Cow::Borrowed(_) => text,
        Cow::Owned(normal) => Cow::Owned(normal),
    }
}

/// The name a Python identifier node (or a dotted name's part, an attribute) denotes.
pub(crate) fn python_identifier<'r, D: Doc>(node: &Node<'r, D>) -> Cow<'r, str> {
    python_name(node.text())
}

/// An ECMAScript identifier's name: its text with each `\uXXXX` and `\u{X…}` escape decoded, as
/// espree's `Identifier.name` (and so eslint-scope's variable names) holds it, so `\u{20BB7}`
/// and `𠮷` name one variable. `text` itself where it holds no escape, or one that decodes to no
/// character (a parse error, which ESLint never reads).
pub(crate) fn ecma_name(text: Cow<'_, str>) -> Cow<'_, str> {
    if !text.contains('\\') {
        return text;
    }
    let mut name = String::with_capacity(text.len());
    let mut rest = text.as_ref();
    while let Some(at) = rest.find('\\') {
        name.push_str(&rest[..at]);
        let escape = &rest[at + 1..];
        let Some(digits) = escape.strip_prefix('u') else {
            return text;
        };
        let (hex, after) = match digits.strip_prefix('{') {
            Some(braced) => match braced.split_once('}') {
                Some((hex, after)) => (hex, after),
                None => return text,
            },
            None if digits.len() >= 4 && digits.is_char_boundary(4) => (&digits[..4], &digits[4..]),
            None => return text,
        };
        let decoded = (!hex.is_empty() && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .then(|| u32::from_str_radix(hex, 16).ok())
            .flatten()
            .and_then(char::from_u32);
        let Some(decoded) = decoded else {
            return text;
        };
        name.push(decoded);
        rest = after;
    }
    name.push_str(rest);
    Cow::Owned(name)
}

/// An identifier's text as `dialect`'s family compares names (`Adapter::name`).
pub(crate) fn name_in(dialect: GrammarDialect, text: Cow<'_, str>) -> Cow<'_, str> {
    with_adapter!(Family::of(dialect), A => A::name(text))
}

#[cfg(test)]
mod tests;
