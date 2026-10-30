//! Queries over what rules produce: a regex clause's matches in a file's text (`regex_matches`),
//! `select_when` (`select_when`) and cross-file relations (`crossfile`). AST rules are matched by
//! the grammar matcher (`matcher::run_clause`).

pub(in crate::code_rule_dsl) mod crossfile;
pub(in crate::code_rule_dsl) mod select_when;

use crate::code_rule_dsl::language::{Family, GrammarDialect};
use regex::Regex;
use std::ops::Range;

/// Where a `file.content` regex starts reading a file of `language`: past a leading byte order
/// mark in a file ESLint's ports read (ECMAScript and Vue), whose `SourceCode` strips U+FEFF from
/// the text its rules read; 0 otherwise. Matches are read from there and their offsets moved back
/// by as much, so spans, columns and fixes stay over the file's own bytes, as every AST
/// witness's do.
pub(in crate::code_rule_dsl) fn content_start(
    source: &str,
    language: Option<GrammarDialect>,
) -> usize {
    let eslint =
        language.is_some_and(|language| matches!(Family::of(language), Family::Ecma | Family::Vue));
    if eslint && source.starts_with('\u{feff}') {
        '\u{feff}'.len_utf8()
    } else {
        0
    }
}

/// A regex clause's matches in `haystack`, in order, at most `limit`; and whether the scan finished.
/// It stops unfinished at a match past `limit`, or once this thread's match deadline is exhausted
/// (polled before each match is kept).
pub(in crate::code_rule_dsl) fn regex_matches(
    regex: &Regex,
    haystack: &str,
    limit: usize,
) -> (Vec<Range<usize>>, bool) {
    let mut matches = Vec::new();
    for found in regex.find_iter(haystack) {
        if crate::code_rule_dsl::budget::matcher_budget_exhausted() || matches.len() >= limit {
            return (matches, false);
        }
        matches.push(found.range());
    }
    (matches, true)
}

#[cfg(test)]
mod tests;
