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

use ast_grep_core::{Doc, Node};
use std::ops::Range;

#[rustfmt::skip]
#[path = "biome_registry.generated.rs"]
mod registry;

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
mod tests {
    use super::*;
    use crate::code_rule_dsl::language::GrammarDialect;
    use ast_grep_language::LanguageExt;

    fn read(source: &str) -> BiomeSuppressions {
        let root = GrammarDialect::Tsx.ast_grep(source);
        BiomeSuppressions::read(&root.root(), source)
    }

    fn rules(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    /// The starts of `needle`'s occurrences a problem of `rule` spanning the occurrence keeps
    /// reported.
    fn reported(source: &str, needle: &str, rule: &str) -> Vec<usize> {
        let suppressions = read(source);
        source
            .match_indices(needle)
            .map(|(at, _)| at)
            .filter(|&at| !suppressions.suppresses(&rules(&[rule]), at..at + needle.len()))
            .collect()
    }

    /// The 0-based lines of the `useEffect(` calls `rule` keeps reported.
    fn hooks(source: &str, rule: &str) -> Vec<usize> {
        let suppressions = read(source);
        source
            .match_indices("useEffect(")
            .filter(|(at, _)| !suppressions.suppresses(&rules(&[rule]), *at..at + 12))
            .map(|(at, _)| source[..at].matches('\n').count())
            .collect()
    }

    const HOOK: &str = "lint/correctness/useHookAtTopLevel";

    /// Biome's `suppression` test (`crates/biome_js_analyze/src/suppressions.tests.rs`
    /// l.146-255): the `==` operators `noDoubleEquals` still reports, at Biome's own offsets.
    /// `lint(suspicious/noDoubleEquals)` is the category `lint` with a value: every lint rule.
    #[test]
    fn biome_suppression_test() {
        const SOURCE: &str = "
            function checkSuppressions1(a, b) {
                a == b;
                // biome-ignore lint/suspicious:whole group
                p == f;
                // biome-ignore lint/suspicious/noDoubleEquals: single rule
                j == k;
                /* biome-ignore lint/complexity/useWhile: multiple block comments */ /* biome-ignore lint/suspicious/noDoubleEquals: multiple block comments */
                o == m;
                // biome-ignore lint/complexity/useWhile: multiple line comments
                // biome-ignore lint/suspicious/noDoubleEquals: multiple line comments
                d == x;
                z == v;
            }

            // biome-ignore lint/suspicious/noDoubleEquals: do not suppress warning for the whole function
            function checkSuppressions2(a, b) {
                a == b;
            }

            function checkSuppressions3(a, b) {
                a == b;
                // biome-ignore lint/suspicious: whole group
                a == b;
                // biome-ignore lint/suspicious/noDoubleEquals: single rule
                a == b;
                /* biome-ignore lint/complexity/useWhile: multiple block comments */ /* biome-ignore lint(suspicious/noDoubleEquals): multiple block comments */
                a == b;
                // biome-ignore lint/complexity/useWhile: multiple line comments
                // biome-ignore lint/suspicious/noDoubleEquals: multiple line comments
                a == b;
                a == b;
            }

            // biome-ignore lint(suspicious/noDoubleEquals): do not suppress warning for the whole function
            function checkSuppressions4(a, b) {
                a == b;
            }

            function checkSuppressions5() {
                // biome-ignore format explanation
                // biome-ignore format(:
                // biome-ignore (value): explanation
                // biome-ignore unknown: explanation
            }
        ";
        assert_eq!(
            reported(SOURCE, "==", "lint/suspicious/noDoubleEquals"),
            [67, 651, 845, 932, 1518, 1713]
        );
    }

    /// Biome's `suppression_issue_5562` and
    /// `suppression_range_should_report_when_contains_inner_comment` (l.828-865, l.953-990): a
    /// `biome-ignore` reaches the line after it only, and one inside the function it precedes
    /// leaves it covered.
    #[test]
    fn biome_ignore_covers_the_next_line() {
        let source = "console.log(\"should error\");\n\n// biome-ignore lint/suspicious/noConsole: foo\nconsole.log(\"should be suppressed\");";
        assert_eq!(
            reported(source, "console.log", "lint/suspicious/noConsole"),
            [0]
        );
        let source = "
// biome-ignore lint/complexity/useArrowFunction: single rule
const foo0 = function (bar: string) {
  // biome-ignore lint/style/noParameterAssign: single rule
  bar = 'baz';
};";
        assert!(reported(source, "function (bar: string) {\n  // biome-ignore lint/style/noParameterAssign: single rule\n  bar = 'baz';\n}", "lint/complexity/useArrowFunction").is_empty());
        assert!(reported(source, "bar = 'baz'", "lint/style/noParameterAssign").is_empty());
    }

    /// What a `biome-ignore` covers: from the comment to the end of the next line, so code after
    /// it on its own line too; not the line after a blank line or a plain comment; consecutive
    /// suppression comments (a trailing one included) move on to the line after the last.
    #[test]
    fn biome_ignore_ranges() {
        let source = "\
// biome-ignore lint: x
useEffect(a); useEffect(b);
useEffect(c);
// biome-ignore lint: x

useEffect(d);
// biome-ignore lint: x
// a note
useEffect(e);
/* biome-ignore lint: x */ useEffect(f);
useEffect(g);
useEffect(h);
useEffect(i); // biome-ignore lint: x
useEffect(j);
// biome-ignore lint/correctness: x
useEffect(k); // biome-ignore lint/suspicious: x
useEffect(l);
useEffect(m);
";
        assert_eq!(hooks(source, HOOK), [2, 5, 8, 11, 12, 17]);
    }

    /// A block comment over several lines covers the line after its last; one on a line of its
    /// own covers the next; JSX comments are comments (as `biome lint` 2.5.15 reports both).
    #[test]
    fn block_and_jsx_comments() {
        let source = "\
/**
 * A note.
 * biome-ignore lint/correctness/useHookAtTopLevel: x
 */
useEffect(a);
useEffect(b);
const c = (
  <div>
    {/* biome-ignore lint/correctness/useHookAtTopLevel: x */}
    {useEffect(c)}
    {useEffect(d)}
  </div>
);
";
        assert_eq!(hooks(source, HOOK), [5, 10]);
    }

    /// `lint`, the rule's group and the rule itself suppress it; another group or rule, an
    /// instance (Rerail's violations report none) or a non-lint category does not.
    #[test]
    fn category_granularity() {
        for (category, suppressed) in [
            ("lint", true),
            ("lint/correctness", true),
            ("lint/correctness/useHookAtTopLevel", true),
            (
                "lint/correctness/noUnusedVariables lint/correctness/useHookAtTopLevel",
                true,
            ),
            ("lint/suspicious", false),
            ("lint/correctness/noUnusedVariables", false),
            ("lint/correctness/useHookAtTopLevel(useEffect)", false),
            ("format", false),
            ("assist", false),
            ("lint/plugin", false),
            ("lint/plugin/useEffect", false),
        ] {
            let source = format!("// biome-ignore {category}: x\nuseEffect(a);\n");
            assert_eq!(hooks(&source, HOOK).is_empty(), suppressed, "{category}");
        }
    }

    /// A comment Biome refuses suppresses nothing: an unknown category (with a known one beside
    /// it too), no reason, no colon, an unclosed `(`, a value without category, `biome-ignore`
    /// not spelled in lowercase anywhere in the comment. A category the JavaScript registry lacks
    /// (a CSS rule, a group) is refused alone.
    #[test]
    fn refused_comments() {
        for comment in [
            "// biome-ignore lint/correctness/useHookAtTopLevell: x",
            "// biome-ignore lint/correctness/useHookAtTopLevel linx: x",
            "// biome-ignore lint/correctness/useHookAtTopLevel:",
            "// biome-ignore lint/correctness/useHookAtTopLevel:   ",
            "// biome-ignore lint/correctness/useHookAtTopLevel",
            "// biome-ignore lint/correctness/useHookAtTopLevel x",
            "// biome-ignore lint/correctness/useHookAtTopLevel(x: y",
            "// biome-ignore (value): x",
            "// biome-ignore lint/correctness/ useHookAtTopLevel: x",
            "// BIOME-IGNORE lint: x",
            "// Biome_Ignore lint: x",
            "// biome-ignore-all-start lint: x",
            "// biome-ignore: x",
            "// biome ignore lint: x",
            "// note: biome-ignore lint: x",
        ] {
            let source = format!("{comment}\nuseEffect(a);\n");
            assert_eq!(hooks(&source, HOOK), [1], "{comment}");
        }
        for comment in [
            "// biome-ignore lint/a11y/useGenericFontNames lint/correctness/useHookAtTopLevel: x",
            "// biome-ignore lint/correctness/useHookAtTopLevel: x biome-ignore",
            "/** biome-ignore lint/correctness/useHookAtTopLevel: x */",
            "//biome-ignore lint/correctness/useHookAtTopLevel:x",
            "// biome-ignorelint: x",
            "// biome-ignore\tlint/correctness/useHookAtTopLevel\t: x",
            "// biome-ignore format lint: x",
            "// biome-ignore lint/correctness/useHookAtTopLevel(x)lint: y",
        ] {
            let source = format!("{comment}\nuseEffect(a);\n");
            assert!(hooks(&source, HOOK).is_empty(), "{comment}");
        }
    }

    /// Biome's top-level tests (l.295-673): `biome-ignore-all` before the first token, after a
    /// shebang or other comments, covers the whole file; several in one comment each count; after
    /// code it covers nothing. An instance counts as its rule there.
    #[test]
    fn biome_ignore_all() {
        for prefix in [
            "",
            "#!/usr/bin/env bun\n",
            "/*\n* Top level comment here. It could be a banner or a license comment\n* MIT\n*/\n",
            "#!/usr/bin/env bun\n\n/* Arbitrary comment here\n */\n",
        ] {
            let source = format!(
                "{prefix}/**\n* biome-ignore-all lint/style/useConst: reason\n*/\n\nuseEffect(a);\nlet foo = 2;\n"
            );
            assert!(
                reported(&source, "let foo", "lint/style/useConst").is_empty(),
                "{prefix}"
            );
            assert_eq!(hooks(&source, HOOK).len(), 1, "{prefix}");
        }
        let source = "/**\n* biome-ignore-all lint/style/useConst: reason\n* biome-ignore-all lint/correctness/useHookAtTopLevel(useEffect): reason2\n*/\n\nuseEffect(a);\nlet foo = 2;\n";
        assert!(hooks(source, HOOK).is_empty());
        assert!(reported(source, "let foo", "lint/style/useConst").is_empty());
        let source = "// biome-ignore-all lint: reason\nuseEffect(a);\nuseEffect(b);\n";
        assert!(hooks(source, HOOK).is_empty());
        // A leading byte order mark is a token to Biome: the suppression after it is not at the top.
        let source = "\u{feff}// biome-ignore-all lint: reason\nuseEffect(a);\nuseEffect(b);\n";
        assert_eq!(hooks(source, HOOK), [1, 2]);
        let source = "useEffect(a);\n// biome-ignore-all lint: reason\nuseEffect(b);\n";
        assert_eq!(hooks(source, HOOK), [0, 2]);
        let source = "\"use strict\";\n// biome-ignore-all lint: reason\nuseEffect(b);\n";
        assert_eq!(hooks(source, HOOK), [2]);
    }

    /// Biome's range tests (l.675-826, l.867-912): `biome-ignore-start` covers what it holds
    /// whole until the `-end` of the same category; an `-end` without its start closes nothing,
    /// an unclosed start runs to the end of the file, and the latest open start is the one closed.
    #[test]
    fn biome_ignore_ranges_start_end() {
        let source = "
// biome-ignore-start lint/suspicious/noDoubleEquals: single rule
// biome-ignore-start lint/suspicious/noDebugger: single rule
a == b;
debugger;
// biome-ignore-end lint/suspicious/noDoubleEquals: single rule
a === b;
debugger;
// biome-ignore-end lint/suspicious/noDebugger: single rule
debugger;

        ";
        assert_eq!(
            reported(source, "debugger", "lint/suspicious/noDebugger"),
            [source.rfind("debugger").unwrap()]
        );
        assert!(reported(source, "a == b", "lint/suspicious/noDoubleEquals").is_empty());
        let source = "
// biome-ignore-start lint/suspicious/noDoubleEquals: single rule
// biome-ignore-start lint/style/useConst: single rule
a == b;
let c;
// biome-ignore-end lint/suspicious/noDoubleEquals: single rule
a == b;
let c;
// biome-ignore-end lint/style/useConst: single rule
        ";
        assert_eq!(
            reported(source, "a == b", "lint/suspicious/noDoubleEquals"),
            [source.rfind("a == b").unwrap()]
        );
        assert!(reported(source, "let c", "lint/style/useConst").is_empty());
        for (source, kept) in [
            // A group start is closed by a group end only.
            (
                "// biome-ignore-start lint/correctness: x\nuseEffect(a);\n// biome-ignore-end lint/correctness/useHookAtTopLevel: x\nuseEffect(b);\n",
                vec![],
            ),
            (
                "// biome-ignore-start lint/correctness: x\nuseEffect(a);\n// biome-ignore-end lint/correctness: x\nuseEffect(b);\n",
                vec![3],
            ),
            (
                "// biome-ignore-start lint: x\nuseEffect(a);\n// biome-ignore-end lint: x\nuseEffect(b);\n",
                vec![3],
            ),
            ("// biome-ignore-end lint: x\nuseEffect(a);\n", vec![1]),
            (
                "useEffect(a);\n// biome-ignore-start lint/correctness/useHookAtTopLevel: x\nuseEffect(b);\n",
                vec![0],
            ),
        ] {
            assert_eq!(hooks(source, HOOK), kept, "{source}");
        }
        // A problem the range holds only in part is not covered.
        let source =
            "useEffect(a,\n// biome-ignore-start lint: x\nb);\n// biome-ignore-end lint: x\n";
        let suppressions = read(source);
        let inside = source.find("b)").expect("b");
        assert!(!suppressions.suppresses(&rules(&[HOOK]), 0..inside + 2));
        assert!(suppressions.suppresses(&rules(&[HOOK]), inside..inside + 2));
    }

    /// Cases checked against `biome lint` 2.5.15 itself (`noDebugger` alone on): the debugger
    /// statements it still reports, as 0-based lines.
    #[test]
    fn cases_match_biome() {
        let debuggers = |source: &str| -> Vec<usize> {
            let suppressions = read(source);
            source
                .match_indices("debugger;")
                .filter(|(at, _)| {
                    !suppressions.suppresses(&rules(&["lint/suspicious/noDebugger"]), *at..at + 9)
                })
                .map(|(at, _)| source[..at].matches('\n').count())
                .collect()
        };
        let source = "\
// biome-ignore lint: x
debugger; debugger;
debugger;
// biome-ignore lint: x

debugger;
// biome-ignore lint: x
// a note
debugger;
/* biome-ignore lint: x */ debugger;
debugger;
debugger;
debugger; // biome-ignore lint: x
debugger;
// biome-ignore lint/suspicious: x
debugger; // biome-ignore lint/style: x
debugger;
debugger;
";
        assert_eq!(debuggers(source), [2, 5, 8, 11, 12, 17]);
        for (comment, reported) in [
            ("// biome-ignore lint/suspicious/noDebuggerr: x", true),
            ("// biome-ignore lint/suspicious/noDebugger linx: x", true),
            ("// biome-ignore lint/suspicious/noDebugger:", true),
            ("// biome-ignore lint/suspicious/noDebugger", true),
            ("// biome-ignore lint/suspicious/noDebugger x", true),
            ("// biome-ignore lint/suspicious/noDebugger(x: y", true),
            ("// biome-ignore (value): x", true),
            ("// BIOME-IGNORE lint: x", true),
            ("// Biome_Ignore lint: x", true),
            ("// biome-ignore-all-start lint: x", true),
            ("// biome-ignore: x", true),
            ("// note: biome-ignore lint: x", true),
            ("// biome-ignore lint/suspicious/noDebugger(x): y", true),
            ("// biome-ignore lint/plugin: x", true),
            ("// biome-ignore assist: x", true),
            (
                "// biome-ignore lint/a11y/useGenericFontNames lint/suspicious/noDebugger: x",
                false,
            ),
            ("/** biome-ignore lint/suspicious/noDebugger: x */", false),
            ("//biome-ignore lint/suspicious/noDebugger:x", false),
            ("// biome-ignorelint: x", false),
            ("// biome-ignore\tlint/suspicious/noDebugger\t: x", false),
            ("// biome-ignore format lint: x", false),
            (
                "// biome-ignore lint/suspicious/noDebugger(x)lint: y",
                false,
            ),
            ("// biome-ignore lint/correctness lint/suspicious: x", false),
        ] {
            let source = format!("x();\n{comment}\ndebugger;\n");
            let expected: &[usize] = if reported { &[2] } else { &[] };
            assert_eq!(debuggers(&source), expected, "{comment}");
        }
        for (source, reported) in [
            (
                "#!/usr/bin/env node\n\n/* Arbitrary comment here\n */\n/**\n* biome-ignore-all lint/suspicious/noDebugger(x): reason\n*/\ndebugger;\n",
                vec![],
            ),
            (
                "\"use strict\";\n// biome-ignore-all lint: reason\ndebugger;\n",
                vec![2],
            ),
            (
                "// biome-ignore-start lint/suspicious: x\ndebugger;\n// biome-ignore-end lint/suspicious/noDebugger: x\ndebugger;\n",
                vec![],
            ),
            (
                "// biome-ignore-end lint: x\ndebugger;\n// biome-ignore-start lint/suspicious/noDebugger: x\ndebugger;\n// biome-ignore-start lint: x\ndebugger;\n// biome-ignore-end lint: x\ndebugger;\n",
                vec![1],
            ),
            // Biome's lexer has no HTML-like comments: `<!--` and `-->` are tokens (a parse
            // error), and both statements stay reported.
            (
                "<!-- biome-ignore lint/suspicious/noDebugger: x\ndebugger;\ndebugger;\n",
                vec![1, 2],
            ),
            (
                "--> biome-ignore lint/suspicious/noDebugger: x\ndebugger;\ndebugger;\n",
                vec![1, 2],
            ),
        ] {
            assert_eq!(debuggers(source), reported, "{source}");
        }
    }

    /// Only lint rules of the JavaScript analyzer are rule names `suppress.biome` takes.
    #[test]
    fn lint_rules() {
        assert!(is_lint_rule("lint/correctness/useHookAtTopLevel"));
        assert!(is_lint_rule("lint/nursery/noReactStringRefs"));
        assert!(!is_lint_rule("lint/correctness/useHookAtTopLevell"));
        assert!(!is_lint_rule("lint/correctness"));
        assert!(!is_lint_rule("lint/a11y/useGenericFontNames"));
        assert!(!is_lint_rule("assist/source/organizeImports"));
        assert!(!is_lint_rule("source/organizeImports"));
        assert!(!is_lint_rule("syntax/correctness/noSuperWithoutExtends"));
        // Sorted and without repeats, as the lookups need.
        assert!(
            registry::CATEGORIES
                .windows(2)
                .all(|pair| pair[0] < pair[1])
        );
        assert!(
            registry::JS_RULES
                .windows(2)
                .all(|pair| (pair[0].0, pair[0].1) < (pair[1].0, pair[1].1))
        );
    }

    /// The 0-based lines a rule reporting every `useEffect` call, with `suppress`, keeps on
    /// `source` through the engine.
    fn hooks_reported(source: &str, suppress: serde_json::Value) -> Vec<u64> {
        use crate::code_rule_dsl::test_support::*;
        use serde_json::{Value, json};
        let violation = json!({ "anchor": "witness.span", "code": "HOOK", "message": "hook",
            "suppress": suppress });
        let rule = json!({ "kind": "call_expression",
            "has": { "field": "function", "kind": "identifier", "regex": "^useEffect$" } });
        let document = clause_document(json!({ "id": "hooks", "violation": violation,
            "forbid": { "ast": { "language": "typescript", "rule": rule } } }));
        let compiled = compile_document_with(
            document,
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(&compiled, "tsx", "avatar.tsx", source);
        assert_eq!(result["failures"], json!([]), "{result}");
        result["violations"]
            .as_array()
            .expect("violations")
            .iter()
            .map(|violation| violation["span"]["startLine"].as_u64().expect("line"))
            .collect()
    }

    /// Through the engine: `suppress.biome` drops Dot, under its `biome-ignore`, and keeps Eh,
    /// whose comment has no reason: `biome lint` 2.5.15 reports lines 2, 6 and 7 of this file.
    /// With `oxlint` beside it, Chip goes too.
    #[test]
    fn biome_ignore_suppresses_violations_through_the_engine() {
        use serde_json::json;
        let source = "\
import { useEffect } from \"react\";
// oxlint-disable-next-line react-hooks/rules-of-hooks
function Chip({ x }) { if (x) useEffect(() => {}); return null; }
// biome-ignore lint/correctness/useHookAtTopLevel: intentional
function Dot({ x }) { if (x) useEffect(() => {}); return null; }
// biome-ignore lint/correctness/useHookAtTopLevel
function Eh({ x }) { if (x) useEffect(() => {}); return null; }
function Plain({ x }) { if (x) useEffect(() => {}); return null; }
";
        let biome = json!({ "rules": [HOOK] });
        let oxlint = json!({ "rules": ["rules-of-hooks"] });
        assert_eq!(hooks_reported(source, json!({ "biome": biome })), [2, 6, 7]);
        assert_eq!(
            hooks_reported(source, json!({ "oxlint": oxlint, "biome": biome })),
            [6, 7]
        );
    }
}
