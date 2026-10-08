//! Queries over what rules produce: a regex clause's matches in a file's text (`regex_matches`),
//! `select_when` (`select_when`) and cross-file relations (`crossfile`). AST rules are matched by
//! the grammar matcher (`matcher::run_clause`).

pub(in crate::code_rule_dsl) mod crossfile;
pub(in crate::code_rule_dsl) mod select_when;

use regex::Regex;
use std::ops::Range;

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
