//! Oxlint 1.88.0's disable directives (`crates/oxc_linter/src/disable_directives.rs`'s
//! `DisableDirectivesBuilder` and `DisableDirectives::contains`, which
//! `crates/oxc_linter/src/context/mod.rs`'s `add_diagnostic` asks with the reporting rule's own
//! name, without its plugin), at oxc tag `oxlint_v1.88.0`
//! (`6941b4ed45ead21c45bc2164cb80a180eed85b43`), with `respectEslintDisableDirectives` at its
//! default, `true`.
//!
//! The comments are oxc's (`Semantic::comments`), HTML-like ones (Annex B.1.1) included:
//! `crates/oxc_parser/src/lexer/punctuation.rs`'s `read_left_angle` and `read_minus` leave a
//! `<!--` anywhere, and a `-->` first on its line, to `skip_single_line_comment` as
//! `CommentKind::HtmlOpen` and `HtmlClose` (`byte_handlers.rs`), whose `content_span`
//! (`crates/oxc_ast/src/ast/comment.rs`) starts after the 4- or 3-byte opener, in a script: a
//! `.cjs` or `.cts` file, or a `.js`, `.jsx`, `.ts` or `.tsx` one without module syntax
//! (`ModuleKind::Unambiguous`). Elsewhere (`.mjs`, `.mts`, module syntax) oxc does not parse
//! the file and oxlint reports nothing, so every `html_comment` tree-sitter-javascript's and
//! tree-sitter-typescript's scanners yield is read as one.
//!
//! Ported from oxc (MIT, VoidZero Inc. & Contributors, and Boshen; see THIRD_PARTY_NOTICES)
//! without its unused-directive bookkeeping and fixes.

use super::names;
use crate::code_rule_dsl::json::{object, reject_unknown};
use ast_grep_core::{Doc, Node};
use serde_json::Value;
use std::collections::HashMap;
use std::ops::Range;

/// Oxlint's `oxlint-disable` comments: oxlint's names of the rule (without a plugin), which a
/// directive must name (after its last `/`, or name none).
pub struct Suppressions;

impl super::SuppressionKind for Suppressions {
    const KEY: &'static str = "oxlint";
    const DETAIL: &'static str = "oxlint: oxlint-disable comments are Oxlint's";
    type Spec = Vec<String>;
    fn spec(value: &serde_json::Value, at: &str) -> napi::Result<Vec<String>> {
        spec(value, at)
    }
}

/// `oxlint.rules`: oxlint's names of the rule (without a plugin), which a disable directive must
/// name (after its last `/`, or name none) to suppress the violation.
pub(super) fn spec(oxlint: &Value, at: &str) -> napi::Result<Vec<String>> {
    reject_unknown(object(oxlint, at)?, &["rules"], at)?;
    names(
        oxlint.get("rules"),
        &format!("{at}/rules"),
        rule_name,
        "oxlint rule names",
        "an oxlint rule name (non-empty, without a plugin prefix, commas or white space: rules-of-hooks)",
    )
}

/// A rule name oxlint asks its directives about: its own name, without the plugin (a directive's
/// list splits on commas and white space).
fn rule_name(name: &str) -> bool {
    !name.is_empty() && !name.contains([',', '/']) && !name.contains(char::is_whitespace)
}

/// Where a directive leaves a rule (or, `rule` none, every rule) off: `DisabledRule` over its
/// interval.
#[derive(Debug, PartialEq, Eq)]
struct Interval {
    start: usize,
    stop: usize,
    /// The name the directive lists, plugin prefix included; `None` disables every rule.
    rule: Option<String>,
    /// A `-line` or `-next-line` directive (`is_next_line`): it covers a problem that starts in
    /// the interval, where a block covers one that overlaps it.
    line: bool,
}

/// A directive's kind (`DirectiveKind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Disable,
    DisableNextLine,
    DisableLine,
    Enable,
}

/// A file's oxlint (and ESLint-spelled) disable directives, as the intervals oxlint builds from
/// them.
#[derive(Debug, Default)]
pub(crate) struct OxlintDirectives {
    intervals: Vec<Interval>,
}

impl OxlintDirectives {
    /// Reads every comment of the file `root` is the root of, HTML-like ones included.
    pub(crate) fn read<D: Doc>(root: &Node<'_, D>, source: &str) -> Self {
        Self::build(
            source,
            root.dfs()
                .filter(|node| matches!(node.kind().as_ref(), "comment" | "html_comment"))
                .map(|comment| comment.range()),
        )
    }

    /// `build_impl` over comments given by their outer ranges (`//`, `/* */`, `<!--` or `-->`
    /// included), in source order. Only pairs build a block interval: `disable` with `enable`,
    /// `disable rule` with `enable rule`. An `enable rule` inside a `disable` (or an `enable`
    /// inside a `disable rule`) matches nothing, and the `disable` runs on to the end of the file.
    fn build(source: &str, comments: impl IntoIterator<Item = Range<usize>>) -> Self {
        let mut intervals = Vec::new();
        // The first unmatched `disable`, and each rule's first unmatched `disable rule`: where
        // their comment's content ends.
        let mut disable_all_start: Option<usize> = None;
        let mut disable_start_map: HashMap<String, usize> = HashMap::new();
        for range in comments {
            let Some(content) = content_span(source, range) else {
                continue;
            };
            let text = source[content.clone()].trim_start();
            let Some((kind, rule_list)) = match_directive(text) else {
                continue;
            };
            let rules = collect_rule_names(rule_list);
            // A `-line` or `-next-line` directive's interval.
            let line = match kind {
                Kind::Disable => {
                    if rules.is_empty() {
                        disable_all_start.get_or_insert(content.end);
                    }
                    for rule in &rules {
                        disable_start_map
                            .entry((*rule).to_owned())
                            .or_insert(content.end);
                    }
                    None
                }
                Kind::DisableNextLine => {
                    // The rest of the comment's line, and the next line up to its line break.
                    let mut stop = content.end;
                    let mut after = source[content.end..].split_inclusive('\n');
                    if let Some(rest_of_line) = after.next() {
                        stop += rest_of_line.len();
                    }
                    if let Some(next_line) = after.next() {
                        stop += next_line.trim_end_matches(['\n', '\r']).len();
                    }
                    Some(content.end..stop)
                }
                Kind::DisableLine => {
                    // From the start of the line the comment starts on to the comment: a block
                    // comment over several lines covers its first line.
                    let start = source[..content.start]
                        .lines()
                        .next_back()
                        .map_or(0, |line| content.start - line.len());
                    Some(start..content.start)
                }
                Kind::Enable => {
                    if rules.is_empty()
                        && let Some(start) = disable_all_start.take()
                    {
                        intervals.push(Interval {
                            start,
                            stop: content.start,
                            rule: None,
                            line: false,
                        });
                    }
                    for rule in &rules {
                        if let Some(start) = disable_start_map.remove(*rule) {
                            intervals.push(Interval {
                                start,
                                stop: content.start,
                                rule: Some((*rule).to_owned()),
                                line: false,
                            });
                        }
                    }
                    None
                }
            };
            if let Some(line) = line {
                let rules = if rules.is_empty() {
                    vec![None]
                } else {
                    rules.iter().map(|rule| Some((*rule).to_owned())).collect()
                };
                intervals.extend(rules.into_iter().map(|rule| Interval {
                    start: line.start,
                    stop: line.end,
                    rule,
                    line: true,
                }));
            }
        }
        // A lone `disable` (or `disable rule`) runs to the end of the file.
        if let Some(start) = disable_all_start {
            intervals.push(Interval {
                start,
                stop: source.len(),
                rule: None,
                line: false,
            });
        }
        for (rule, start) in disable_start_map {
            intervals.push(Interval {
                start,
                stop: source.len(),
                rule: Some(rule),
                line: false,
            });
        }
        Self { intervals }
    }

    /// Whether a problem of a rule named any of `rules` (oxlint's names, without a plugin),
    /// spanning bytes `span`, is suppressed (`contains`): an interval that overlaps the span
    /// (`Lapper::find`) disables every rule or a listed name whose part after its last `/` is the
    /// rule's name; a `-line` or `-next-line` interval must also hold the span's start.
    pub(crate) fn suppresses(&self, rules: &[String], span: Range<usize>) -> bool {
        self.intervals.iter().any(|interval| {
            let found = interval.start < span.end && interval.stop > span.start;
            let names = interval.rule.as_deref().is_none_or(|name| {
                let bare = name.rsplit_once('/').map_or(name, |(_, rule)| rule);
                rules.iter().any(|rule| rule == bare)
            });
            let covered = if interval.line {
                span.start >= interval.start && span.start < interval.stop
            } else {
                span.start < interval.stop && span.end > interval.start
            };
            found && names && covered
        })
    }
}

/// A comment's `content_span`: inside its `//` or `/* */`, or after its `<!--` or `-->`.
fn content_span(source: &str, range: Range<usize>) -> Option<Range<usize>> {
    let text = &source[range.clone()];
    if text.starts_with("//") {
        Some(range.start + 2..range.end)
    } else if text.starts_with("/*") && text.ends_with("*/") && text.len() >= 4 {
        Some(range.start + 2..range.end - 2)
    } else if text.starts_with("<!--") {
        Some(range.start + 4..range.end)
    } else if text.starts_with("-->") {
        Some(range.start + 3..range.end)
    } else {
        None
    }
}

/// `match_directive`: an `eslint-` or `oxlint-` `disable`, `disable-next-line`, `disable-line` or
/// `enable` at the start of the (start-trimmed) comment text, ending the text or followed by
/// white space, and the text after it.
fn match_directive(text: &str) -> Option<(Kind, &str)> {
    fn is_directive_end(text: &str) -> bool {
        text.is_empty() || text.starts_with(char::is_whitespace)
    }
    let rest = text
        .strip_prefix("eslint-")
        .or_else(|| text.strip_prefix("oxlint-"))?;
    if let Some(rest) = rest.strip_prefix("disable") {
        if let Some(rest) = rest.strip_prefix("-next-line")
            && is_directive_end(rest)
        {
            return Some((Kind::DisableNextLine, rest));
        } else if let Some(rest) = rest.strip_prefix("-line")
            && is_directive_end(rest)
        {
            return Some((Kind::DisableLine, rest));
        } else if is_directive_end(rest) {
            return Some((Kind::Disable, rest));
        }
    } else if let Some(rest) = rest.strip_prefix("enable")
        && is_directive_end(rest)
    {
        return Some((Kind::Enable, rest));
    }
    None
}

/// `collect_rule_names`: names split on commas and white space, up to a description: `--`, or a
/// `-` between white space.
fn collect_rule_names(text: &str) -> Vec<&str> {
    let mut rules = Vec::new();
    let mut rule_start = None;
    let mut rule_end = text.len();
    let mut chars = text.char_indices().peekable();
    let mut previous = None;
    while let Some((index, ch)) = chars.next() {
        let is_description_start = ch == '-'
            && chars.peek().map(|(_, next)| *next).is_some_and(|next| {
                next == '-' || (previous.is_some_and(char::is_whitespace) && next.is_whitespace())
            });
        if is_description_start {
            rule_end = index;
            break;
        }
        if ch == ',' || ch.is_whitespace() {
            if let Some(start) = rule_start.take() {
                rules.push(&text[start..index]);
            }
        } else if rule_start.is_none() {
            rule_start = Some(index);
        }
        previous = Some(ch);
    }
    if let Some(start) = rule_start {
        rules.push(&text[start..rule_end]);
    }
    rules
}

#[cfg(test)]
mod tests;
