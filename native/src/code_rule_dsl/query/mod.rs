//! Queries over what rules produce: a regex clause's matches in a file's text (`regex_matches`),
//! `select_when` (`select_when`) and cross-file relations (`crossfile`). AST rules are matched by
//! the grammar matcher (`matcher::run_clause`).

pub(in crate::code_rule_dsl) mod crossfile;
mod line_state;
pub(in crate::code_rule_dsl) mod select_when;

use super::model::RegexOptions;
use regex::Regex;
use std::ops::Range;

/// A match local to its context (the complete source or one independent line).
pub(in crate::code_rule_dsl) struct RegexMatch {
    pub(in crate::code_rule_dsl) range: Range<usize>,
    pub(in crate::code_rule_dsl) context: Range<usize>,
    pub(in crate::code_rule_dsl) line: Option<usize>,
}

pub(in crate::code_rule_dsl) fn regex_query_matches(
    regex: &Regex,
    haystack: &str,
    options: &RegexOptions,
    limit: usize,
) -> (Vec<RegexMatch>, bool) {
    if !options.content_include.iter().all(|r| r.is_match(haystack))
        || options.content_exclude.iter().any(|r| r.is_match(haystack))
    {
        return (Vec::new(), true);
    }
    let mut matches = Vec::new();
    let mut state = line_state::State::new(options.lines.as_ref());
    let mut scan = |context: Range<usize>, line: Option<usize>| -> bool {
        if crate::code_rule_dsl::budget::matcher_budget_exhausted() {
            return false;
        }
        let text = &haystack[context.clone()];
        if let Some(lines) = &options.lines {
            match state.allows(lines, text) {
                Ok(false) => return true,
                Err(()) => return false,
                Ok(true) => {}
            }
        }
        if options.lines.as_ref().is_some_and(|lines| {
            !lines.include.iter().all(|regex| regex.is_match(text))
                || lines.exclude.iter().any(|regex| regex.is_match(text))
        }) {
            return true;
        }
        for found in regex.find_iter(text) {
            if crate::code_rule_dsl::budget::matcher_budget_exhausted() {
                return false;
            }
            if !options
                .include
                .iter()
                .all(|regex| regex.is_match(found.as_str()))
                || options
                    .exclude
                    .iter()
                    .any(|regex| regex.is_match(found.as_str()))
            {
                continue;
            }
            if matches.len() >= limit {
                return false;
            }
            matches.push(RegexMatch {
                range: found.range(),
                context: context.clone(),
                line,
            });
            if options.lines.as_ref().is_some_and(|lines| lines.first) {
                break;
            }
        }
        true
    };
    let complete = if options.lines.is_some() {
        let mut start = 0;
        let mut line = 0;
        let mut chars = haystack.char_indices().peekable();
        let mut complete = true;
        while let Some((at, ch)) = chars.next() {
            if ch == '\n'
                || (!options.lines.as_ref().is_some_and(|l| l.lf_only)
                    && matches!(
                        ch,
                        '\n' | '\r'
                            | '\u{b}'
                            | '\u{c}'
                            | '\u{1c}'
                            | '\u{1d}'
                            | '\u{1e}'
                            | '\u{85}'
                            | '\u{2028}'
                            | '\u{2029}'
                    ))
            {
                if !scan(start..at, Some(line)) {
                    complete = false;
                    break;
                }
                start = at + ch.len_utf8();
                if ch == '\r' && chars.peek().is_some_and(|(_, ch)| *ch == '\n') {
                    start += 1;
                    chars.next();
                }
                line += 1;
            }
        }
        complete && (start == haystack.len() || scan(start..haystack.len(), Some(line)))
    } else {
        scan(0..haystack.len(), None)
    };
    (matches, complete)
}

/// A regex clause's matches in `haystack`, in order, at most `limit`; and whether the scan finished.
/// It stops unfinished at a match past `limit`, or once this thread's match deadline is exhausted
/// (polled before each match is kept).
#[cfg(test)]
pub(in crate::code_rule_dsl) fn regex_matches(
    regex: &Regex,
    haystack: &str,
    limit: usize,
) -> (Vec<Range<usize>>, bool) {
    let (matches, complete) = regex_query_matches(regex, haystack, &RegexOptions::default(), limit);
    (
        matches.into_iter().map(|matched| matched.range).collect(),
        complete,
    )
}

#[cfg(test)]
mod tests;
