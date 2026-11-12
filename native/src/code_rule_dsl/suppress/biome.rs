//! Biome 2.5.15's suppression comments as its JavaScript analyzer reads them, at tag
//! `@biomejs/biome@2.5.15` (`0b4c11f553a7e702f9851af139662e1de45dc131`): `biome-ignore`,
//! `biome-ignore-all` and `biome-ignore-start`/`biome-ignore-end`, for lint rules.
//!
//! - The comment (`crates/biome_suppression/src/lib.rs`): each line of it
//!   (`parse_suppression_comment_with_line_prefix`, l.106-212) that starts, past white space and a
//!   block comment's `*`s, with `biome-ignore` (case-insensitive, `-` or `_`), then `-all`,
//!   `-start` or `-end`, and whose comment holds that kind's lowercase spelling somewhere
//!   (`base.find`, l.196-200), is one suppression: categories separated by white space, each
//!   optionally `(value)`, then `:` and a non-empty reason (`parse_suppression_line`,
//!   l.348-433). A category must be a Biome diagnostic category or `lint/plugin/<name>`
//!   (`parse_category`, l.445-462). Any error refuses the whole line: a missing colon, reason or
//!   `)`, an unknown category.
//! - Each category is one suppression (`to_analyzer_suppressions`,
//!   `crates/biome_analyze/src/lib.rs` l.742-801): `lint` every lint rule, `lint/<group>` a group's,
//!   `lint/<group>/<rule>` one rule's (with `(value)`, one instance of it), `assist`, `assist/..`,
//!   `syntax/..` and `lint/plugin` what is not a lint rule, any other category nothing. A group or
//!   rule the JavaScript analyzer's registry lacks refuses that suppression alone
//!   (`Suppressions::map_to_rule_filter`, `crates/biome_analyze/src/suppressions.rs` l.564-607).
//! - Its scope (`PhaseRunner::handle_token`, `handle_comment` and `bump_line_index`,
//!   `crates/biome_analyze/src/lib.rs` l.437-505 and l.552-623, over `suppressions.rs`): a
//!   `biome-ignore` belongs to the line after the comment's last line, and spans from the comment
//!   to what the analyzer reads on that line (`push_line_suppression` l.525-561, `expand_range`
//!   l.663-677, `overlap_last_suppression` l.684-715); suppression comments on consecutive lines
//!   move on together. A problem starting in that span is suppressed (`suppresses`, l.442-523).
//!   `biome-ignore-all` before the file's first token (a shebang aside,
//!   `JsSyntaxKind::is_allowed_before_suppressions`, `crates/biome_js_syntax/src/lib.rs`
//!   l.168-170) covers the file, and elsewhere nothing (`TopLevelSuppression::push_suppression`,
//!   l.47-78). `biome-ignore-start` runs until a `biome-ignore-end` of the same category closes
//!   it (the latest open one), or to the end of the file, and covers a problem it holds whole
//!   (`RangeSuppressions::push_suppression` l.233-326, `suppress_rule` l.329-352).
//! - The analyzer's tokens, white space and line breaks are the JavaScript lexer's
//!   (`crates/biome_js_parser/src/lexer/mod.rs`: `consume_newline` l.661-680, `read_slash`
//!   l.1584-1652, `is_linebreak` l.2273-2275): a block comment counts as several lines only if it
//!   holds a line break, a `//` comment never.
//!
//! The diagnostic categories and the analyzer's registry are `biome_registry.generated.rs`.
//! Ported from Biome (MIT, Biome Developers and Contributors; see THIRD_PARTY_NOTICES) without its
//! diagnostics, unused-suppression bookkeeping, plugins' own suppressions and embedded snippets. A
//! comment Biome's analyzer panics on (`/* BIOME-IGNORE lint: x, biome-ignore */`, whose reason
//! range runs past the comment) is read as its parser reads it.

use super::names;
use crate::code_rule_dsl::json::{object, reject_unknown};
use ast_grep_core::{Doc, Node};
use serde_json::Value;
use std::ops::Range;

/// Biome's `biome-ignore` comments: the lint rules (`lint/<group>/<rule>`) a comment must
/// name, or their group, or `lint`.
pub struct Suppressions;

impl super::SuppressionKind for Suppressions {
    const KEY: &'static str = "biome";
    const DETAIL: &'static str = "biome: biome-ignore comments are Biome's";
    type Spec = Vec<String>;
    fn spec(value: &serde_json::Value, at: &str) -> napi::Result<Vec<String>> {
        spec(value, at)
    }
}

/// `biome.rules`: the lint rules (`lint/<group>/<rule>`) a `biome-ignore` comment must name, or
/// their group, or `lint`, to suppress the violation.
pub(super) fn spec(biome: &Value, at: &str) -> napi::Result<Vec<String>> {
    reject_unknown(object(biome, at)?, &["rules"], at)?;
    names(
        biome.get("rules"),
        &format!("{at}/rules"),
        is_lint_rule,
        "Biome lint rule names",
        "a lint rule of Biome 2.5.15's JavaScript analyzer (lint/<group>/<rule>: lint/correctness/useHookAtTopLevel)",
    )
}

#[rustfmt::skip]
#[path = "biome_registry.generated.rs"]
pub mod registry;

/// A suppression's `RuleCategory`: lint rules are the ones a violation names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Category {
    Lint,
    Action,
    Syntax,
}

/// A `RuleFilter` of the registry: a group, or one of its rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Filter {
    Group(&'static str),
    Rule(&'static str, &'static str),
}

impl Filter {
    /// `RuleFilter == RuleKey` (`crates/biome_analyze/src/matcher.rs` l.132-139).
    fn matches(self, group: &str, rule: &str) -> bool {
        match self {
            Self::Group(own) => own == group,
            Self::Rule(own_group, own_rule) => own_group == group && own_rule == rule,
        }
    }
}

/// What one suppression covers.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Target {
    /// `lint` or `assist`: every rule of the category (`suppressed_categories`).
    Everything(Category),
    /// A group or rule; `instance` when it names one instance, `rule(value)`.
    Rule(Category, Filter, bool),
    /// `lint/plugin` (`None`) or `lint/plugin/<name>`.
    Plugin(Option<String>),
}

impl Target {
    /// Whether it covers a lint problem of any of `keys` (group, rule). A line suppression of one
    /// instance covers only a problem reporting that instance, and Rerail's violations report none.
    fn covers(&self, keys: &[(&str, &str)], line: bool) -> bool {
        match self {
            Self::Everything(category) => *category == Category::Lint,
            Self::Rule(Category::Lint, filter, instance) => {
                !(line && *instance) && keys.iter().any(|(group, rule)| filter.matches(group, rule))
            }
            Self::Rule(..) | Self::Plugin(_) => false,
        }
    }
}

/// A suppression comment's kind (`SuppressionKind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Classic,
    All,
    RangeStart,
    RangeEnd,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Classic => "biome-ignore",
            Self::All => "biome-ignore-all",
            Self::RangeStart => "biome-ignore-start",
            Self::RangeEnd => "biome-ignore-end",
        }
    }
}

/// One category of a parsed comment line, before the registry is asked about it
/// (`AnalyzerSuppressionKind`).
#[derive(Debug, PartialEq, Eq)]
enum Named<'a> {
    Everything(Category),
    /// The category's name past `lint/`, `assist/` or `syntax/`, and its `(value)`.
    Rule(Category, &'a str, Option<&'a str>),
    Plugin(Option<&'a str>),
}

/// A `biome-ignore` (`LineSuppression`): the line it applies to and the span it covers.
#[derive(Debug)]
struct LineSuppression {
    line_index: usize,
    text_range: Range<usize>,
    target: Target,
}

/// A `biome-ignore-start` (`RangeSuppression`) and whether an `-end` closed it.
#[derive(Debug)]
struct RangeSuppression {
    target: Target,
    range: Range<usize>,
    ended: bool,
}

/// A file's Biome suppressions, as the analyzer has them once it has read every token.
#[derive(Debug, Default)]
pub(crate) struct BiomeSuppressions {
    /// `biome-ignore-all` at the top of the file: they cover all of it.
    top_level: Vec<Target>,
    ranges: Vec<RangeSuppression>,
    lines: Vec<LineSuppression>,
}

impl BiomeSuppressions {
    /// Reads the file `root` is the root of: its comments, and the tokens and line breaks between
    /// them.
    pub(crate) fn read<D: Doc>(root: &Node<'_, D>, source: &str) -> Self {
        let mut comments = root
            .dfs()
            .filter(|node| node.kind() == "comment")
            .map(|comment| comment.range())
            .collect::<Vec<_>>();
        comments.sort_by_key(|range| range.start);
        Self::build(source, &comments)
    }

    /// The analyzer's first phase over `source` with `comments` (outer ranges, in order): line
    /// breaks, tokens and comments in source order.
    fn build(source: &str, comments: &[Range<usize>]) -> Self {
        let mut reader = Reader {
            source,
            suppressions: Self::default(),
            line_index: 0,
            deny_top_level: false,
        };
        let mut at = 0;
        // A shebang is a token the top-level suppressions may follow.
        if source.starts_with("#!") {
            at = source.find(is_linebreak).unwrap_or(source.len());
            reader.token(0..at, true);
        }
        for comment in comments {
            if comment.start < at {
                continue;
            }
            reader.between(at..comment.start);
            reader.comment(comment.clone());
            at = comment.end;
        }
        reader.between(at..source.len());
        reader.suppressions
    }

    /// Whether a problem of any of `rules` (`lint/<group>/<rule>`) spanning `span` is suppressed
    /// (`Suppressions::suppresses`): by a `biome-ignore-all`, by a `biome-ignore-start` range
    /// holding it, or by a `biome-ignore` whose span holds its start.
    pub(crate) fn suppresses(&self, rules: &[String], span: Range<usize>) -> bool {
        let keys = rules
            .iter()
            .filter_map(|rule| rule.strip_prefix("lint/")?.split_once('/'))
            .collect::<Vec<_>>();
        self.top_level
            .iter()
            .any(|target| target.covers(&keys, false))
            || self.ranges.iter().any(|range| {
                range.range.start <= span.start
                    && span.end <= range.range.end
                    && range.target.covers(&keys, false)
            })
            || self.lines.iter().any(|line| {
                line.text_range.contains(&span.start) && line.target.covers(&keys, true)
            })
    }
}

/// Whether `name` (`lint/<group>/<rule>`) is a lint rule of Biome 2.5.15's JavaScript analyzer.
pub(crate) fn is_lint_rule(name: &str) -> bool {
    name.strip_prefix("lint/")
        .and_then(|name| name.split_once('/'))
        .and_then(|(group, rule)| find_rule(group, rule))
        .is_some_and(|index| registry::JS_RULES[index].2 == "lint")
}

/// `MetadataRegistry::find_rule`: the registry entry of `group`'s `rule`.
fn find_rule(group: &str, rule: &str) -> Option<usize> {
    registry::JS_RULES
        .binary_search_by(|(own_group, own_rule, _)| (*own_group, *own_rule).cmp(&(group, rule)))
        .ok()
}

/// `MetadataRegistry::find_group`: the group as the registry spells it, if any rule is in it.
fn find_group(group: &str) -> Option<&'static str> {
    let at = registry::JS_RULES.partition_point(|(own, _, _)| *own < group);
    registry::JS_RULES
        .get(at)
        .filter(|(own, _, _)| *own == group)
        .map(|(own, _, _)| *own)
}

/// `is_linebreak` of Biome's JavaScript lexer.
fn is_linebreak(chr: char) -> bool {
    matches!(chr, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// The analyzer reading a file's tokens: the line it is on and the suppressions so far.
struct Reader<'s> {
    source: &'s str,
    suppressions: BiomeSuppressions,
    line_index: usize,
    /// A token that may not precede `biome-ignore-all` has been read.
    deny_top_level: bool,
}

impl Reader<'_> {
    /// Text between comments: line breaks (one per piece, `\r\n` as one), white space, and the
    /// runs of other characters, which are (parts of) tokens.
    fn between(&mut self, range: Range<usize>) {
        let text = &self.source[range.clone()];
        let mut chars = text.char_indices().peekable();
        while let Some((index, chr)) = chars.next() {
            let start = range.start + index;
            if is_linebreak(chr) {
                let mut end = start + chr.len_utf8();
                if chr == '\r' && chars.next_if(|(_, next)| *next == '\n').is_some() {
                    end += 1;
                }
                self.bump(start..end);
            } else if chr == '\u{feff}' && start == 0 {
                // Biome lexes a file's leading byte order mark as a token: a `biome-ignore-all` after it is not at
                // the top (`biome lint` 2.5.15 reports `suppressions/incorrect` for it).
                self.token(start..start + chr.len_utf8(), false);
            } else if !(chr.is_whitespace() || chr == '\u{feff}') {
                let mut end = start + chr.len_utf8();
                while let Some((index, chr)) = chars.next_if(|(_, chr)| {
                    !(is_linebreak(*chr) || chr.is_whitespace() || *chr == '\u{feff}')
                }) {
                    end = range.start + index + chr.len_utf8();
                }
                self.token(start..end, false);
            }
        }
    }

    /// A token: its text counts lines, then it may end the file's top.
    fn token(&mut self, range: Range<usize>, shebang: bool) {
        self.bump(range.clone());
        if !self.deny_top_level && !range.is_empty() {
            self.deny_top_level = !shebang;
        }
    }

    /// A comment piece: a block comment holding a line break counts its lines first, then each of
    /// its suppressions is registered (`handle_comment`).
    fn comment(&mut self, range: Range<usize>) {
        let text = &self.source[range.clone()];
        if text.starts_with("/*") && text[2..].contains(is_linebreak) {
            self.bump(range.clone());
        }
        let is_leading_in_file = !self.deny_top_level;
        for (kind, named) in parse_comment(text) {
            for named in named {
                if self.push(kind, named, range.clone(), is_leading_in_file)
                    && kind == Kind::Classic
                {
                    // Expand scope of preceding line comments.
                    self.overlap_last_suppression(self.line_index + 1, &range);
                }
            }
        }
    }

    /// `bump_line_index`: each `\n` in the piece ends the current line, after the piece's text up
    /// to it extends what reads the current line; a piece that extends nothing that way extends
    /// what reads the line it ends on.
    fn bump(&mut self, range: Range<usize>) {
        let text = &self.source[range.clone()];
        let mut did_match = false;
        for (index, _) in text.match_indices('\n') {
            did_match = self.expand_range(range.start..range.start + index, self.line_index);
            self.line_index += 1;
        }
        if !did_match {
            self.expand_range(range, self.line_index);
        }
    }

    /// `Suppressions::expand_range`: open ranges cover `range`, and so do the last line
    /// suppressions if they read line `line_index`.
    fn expand_range(&mut self, range: Range<usize>, line_index: usize) -> bool {
        for open in self
            .suppressions
            .ranges
            .iter_mut()
            .filter(|open| !open.ended)
        {
            cover(&mut open.range, &range);
        }
        let mut found = false;
        for line in self.suppressions.lines.iter_mut().rev() {
            if line.line_index != line_index {
                break;
            }
            cover(&mut line.text_range, &range);
            found = true;
        }
        found
    }

    /// `overlap_last_suppression`: the last line suppressions on this line or the one before move
    /// to line `next_line_index`, covering the comment.
    fn overlap_last_suppression(&mut self, next_line_index: usize, range: &Range<usize>) {
        for line in self.suppressions.lines.iter_mut().rev() {
            if line.line_index != next_line_index && line.line_index + 1 != next_line_index {
                break;
            }
            line.line_index = next_line_index;
            cover(&mut line.text_range, range);
        }
    }

    /// `Suppressions::push_suppression`: false when Biome refuses it (an unknown group or rule, a
    /// `biome-ignore-all` after the file's top, an `-end` closing nothing).
    fn push(&mut self, kind: Kind, named: Named<'_>, range: Range<usize>, leading: bool) -> bool {
        let target = match named {
            Named::Everything(category) => Target::Everything(category),
            Named::Plugin(name) => Target::Plugin(name.map(str::to_owned)),
            Named::Rule(category, name, value) => {
                let filter = match name.split_once('/') {
                    None => find_group(name).map(Filter::Group),
                    Some((group, rule)) => find_rule(group, rule).map(|index| {
                        let (group, rule, _) = registry::JS_RULES[index];
                        Filter::Rule(group, rule)
                    }),
                };
                let Some(filter) = filter else {
                    return false;
                };
                Target::Rule(category, filter, value.is_some())
            }
        };
        let suppressions = &mut self.suppressions;
        match kind {
            Kind::Classic => suppressions.lines.push(LineSuppression {
                line_index: self.line_index,
                text_range: range,
                target,
            }),
            Kind::All if !leading => return false,
            Kind::All => suppressions.top_level.push(target),
            Kind::RangeStart => suppressions.ranges.push(RangeSuppression {
                target,
                range,
                ended: false,
            }),
            Kind::RangeEnd => {
                // The latest open start of the same category and filter (an instance aside).
                let same = |open: &Target| match (open, &target) {
                    (Target::Rule(category, filter, _), Target::Rule(own, own_filter, _)) => {
                        category == own && filter == own_filter
                    }
                    (open, target) => open == target,
                };
                let Some(open) = suppressions
                    .ranges
                    .iter_mut()
                    .rev()
                    .find(|open| !open.ended && same(&open.target))
                else {
                    return false;
                };
                open.ended = true;
                cover(&mut open.range, &range);
            }
        }
        true
    }
}

/// `TextRange::cover`.
fn cover(range: &mut Range<usize>, other: &Range<usize>) {
    range.start = range.start.min(other.start);
    range.end = range.end.max(other.end);
}

/// `parse_suppression_comment` then `to_analyzer_suppressions`: the suppressions of each line of
/// the comment Biome reads without error, with their categories.
fn parse_comment(base: &str) -> Vec<(Kind, Vec<Named<'_>>)> {
    let block = base.starts_with("/*");
    if !block && !base.starts_with("//") {
        return Vec::new();
    }
    let mut comment = &base[2..];
    if block {
        comment = comment
            .strip_suffix("*/")
            .or_else(|| comment.strip_suffix(['*', '/']))
            .unwrap_or(comment);
    }
    comment
        .lines()
        .filter_map(|line| {
            let mut line = line.trim_start();
            if block {
                line = line.trim_start_matches('*').trim_start();
            }
            if line.is_empty() {
                return None;
            }
            // `/biome[-_]ignore/i`.
            for pattern in [
                ['b', 'B'],
                ['i', 'I'],
                ['o', 'O'],
                ['m', 'M'],
                ['e', 'E'],
                ['-', '_'],
                ['i', 'I'],
                ['g', 'G'],
                ['n', 'N'],
                ['o', 'O'],
                ['r', 'R'],
                ['e', 'E'],
            ] {
                line = line.strip_prefix(pattern)?;
            }
            let mut kind = Kind::Classic;
            for (patterns, of) in [
                (["-ALL", "-all"], Kind::All),
                (["-START", "-start"], Kind::RangeStart),
                (["-END", "-end"], Kind::RangeEnd),
            ] {
                for pattern in patterns {
                    if let Some(rest) = line.strip_prefix(pattern) {
                        kind = of;
                        line = rest;
                    }
                }
            }
            if !base.contains(kind.as_str()) {
                return None;
            }
            let categories = parse_suppression_line(line.trim_start()).ok()?;
            Some((kind, categories.into_iter().filter_map(named).collect()))
        })
        .collect()
}

/// A comment line's categories: (category, plugin name, value).
type Categories<'a> = Vec<(&'static str, Option<&'a str>, Option<&'a str>)>;

/// `parse_suppression_line`: `{ <category> { (<value>) }? }+: <reason>`, as (category,
/// plugin name, value) triples; `Err` where Biome reports a diagnostic and suppresses nothing.
fn parse_suppression_line(base: &str) -> Result<Categories<'_>, ()> {
    let mut line = base;
    let mut categories = Vec::new();
    loop {
        let separator = line
            .find(|chr: char| chr == ':' || chr == '(' || chr.is_whitespace())
            .ok_or(())?;
        let (category, rest) = line.split_at(separator);
        let (category, plugin) = parse_category(category.trim_end())?;
        // Biome splits one byte off: a separator of several bytes (non-ASCII white space) makes it
        // panic, so it reads no suppression there.
        if !rest.is_char_boundary(1) {
            return Err(());
        }
        let (separator, rest) = rest.split_at(1);
        match separator {
            ":" => {
                if let Some(category) = category {
                    categories.push((category, plugin, None));
                }
                line = rest.trim_start();
                break;
            }
            "(" => {
                let category = category.ok_or(())?;
                let paren = rest.find(')').ok_or(())?;
                let (value, rest) = rest.split_at(paren);
                categories.push((category, plugin, Some(value.trim())));
                line = rest[1..].trim_start();
            }
            _ => {
                if let Some(category) = category {
                    categories.push((category, plugin, None));
                }
                line = rest.trim_start();
            }
        }
    }
    if line.trim_end().is_empty() {
        return Err(());
    }
    Ok(categories)
}

/// `parse_category`: none, `lint/plugin` with a plugin's name, or a Biome diagnostic category.
fn parse_category(category: &str) -> Result<(Option<&'static str>, Option<&str>), ()> {
    if category.is_empty() {
        return Ok((None, None));
    }
    if let Some(plugin) = category.strip_prefix("lint/plugin/") {
        return Ok((Some("lint/plugin"), Some(plugin)));
    }
    registry::CATEGORIES
        .binary_search(&category)
        .map(|index| (Some(registry::CATEGORIES[index]), None))
        .map_err(|_| ())
}

/// `to_analyzer_suppressions`, for one category: what it suppresses, if anything.
fn named<'a>(
    (category, plugin, value): (&'static str, Option<&'a str>, Option<&'a str>),
) -> Option<Named<'a>> {
    match category {
        "lint" => Some(Named::Everything(Category::Lint)),
        "assist" => Some(Named::Everything(Category::Action)),
        "lint/plugin" => Some(Named::Plugin(plugin)),
        _ => {
            if let Some(rule) = category.strip_prefix("lint/") {
                Some(Named::Rule(Category::Lint, rule, value))
            } else if let Some(action) = category.strip_prefix("assist/") {
                Some(Named::Rule(Category::Action, action, None))
            } else {
                let rule = category.strip_prefix("syntax/")?;
                Some(Named::Rule(Category::Syntax, rule, None))
            }
        }
    }
}

#[cfg(test)]
mod tests;
