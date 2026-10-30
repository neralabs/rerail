//! Queries over what rules produce: a regex clause's matches in a file's text (`regex_matches`),
//! `select_when` (`select_when`) and cross-file relations (`crossfile`). AST rules are matched by
//! the grammar matcher (`matcher::run_clause`).

pub(in crate::code_rule_dsl) mod crossfile;
pub(in crate::code_rule_dsl) mod select_when;

use crate::code_rule_dsl::language::adapter::strips_bom;
use crate::code_rule_dsl::model::{FileInput, RegexSource};
use regex::Regex;
use std::ops::Range;

/// The text a regex clause reading `source` scans in `file`, and the offset of its start in what
/// it was cut from. A `file.content` regex reads past a leading byte order mark the file's family
/// strips (`Adapter::STRIPS_BOM`: ESLint's `SourceCode` does); a match's offsets are moved back by
/// as much, so spans, columns and fixes stay over the file's own bytes, as every AST witness's do.
pub(in crate::code_rule_dsl) fn regex_haystack<'a>(
    source: &RegexSource,
    file: &'a FileInput,
) -> (&'a str, usize) {
    match source {
        RegexSource::Content => {
            let start = if strips_bom(&file.language) && file.source.starts_with('\u{feff}') {
                '\u{feff}'.len_utf8()
            } else {
                0
            };
            (&file.source[start..], start)
        }
        RegexSource::Path => (&file.path, 0),
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
