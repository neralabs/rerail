//! Python identifiers as CPython reads them: the tokenizer NFKC-normalizes every identifier
//! (PEP 3131), so `𝒞` and `C` name one binding, attribute, keyword or module. Ruff's lexer does
//! the same. Names the Python family extracts to compare, resolve or qualify go through
//! `python_name`; findings, spans and `regex` keep the source text, and string contents (an
//! `__all__` entry, a `getattr` argument) stay raw, as CPython keeps them. ECMAScript and PHP
//! names are compared as written (ESLint compares code units, PHP bytes).

use crate::policy_dsl::language::{Family, GrammarDialect};
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

/// An identifier's text as `dialect`'s family compares names: NFKC for Python, as written for
/// the others.
pub(crate) fn name_in(dialect: GrammarDialect, text: Cow<'_, str>) -> Cow<'_, str> {
    if Family::of(dialect) == Family::Python {
        python_name(text)
    } else {
        text
    }
}

#[cfg(test)]
mod tests;
