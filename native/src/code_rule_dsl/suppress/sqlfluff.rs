//! SQLFluff 4.4.0's `noqa` directives, following `sqlfluff/core/rules/noqa.py`.

use super::names;
use crate::code_rule_dsl::json::{object, reject_unknown};
use crate::code_rule_dsl::lines::LineTable;
use ast_grep_core::{Doc, Node};
use serde_json::Value;

/// `suppress.sqlfluff`: `{ "rules": [...] }`, the references SQLFluff knows the violation's rule
/// by, the keys of its `RulePack.reference_map` that map to the rule: its code, its name, its
/// legacy aliases and its groups (`["AL05", "L025", "aliasing", "aliasing.unused", "all",
/// "core"]`). A `-- noqa: p` directive suppresses the violation where `p` (a code, name, alias,
/// group or `fnmatch` glob) matches one of them, as SQLFluff's `_parse_noqa` expands `p` over the
/// map's keys.
pub(super) fn spec(sqlfluff: &Value, at: &str) -> napi::Result<Vec<String>> {
    reject_unknown(object(sqlfluff, at)?, &["rules"], at)?;
    names(
        sqlfluff.get("rules"),
        &format!("{at}/rules"),
        reference,
        "rule references",
        "a rule's code, name, alias or group (non-empty, without commas or surrounding white space)",
    )
}

/// A reference a `noqa` list can name: SQLFluff splits the list on commas and strips each.
fn reference(name: &str) -> bool {
    !name.is_empty() && !name.contains(',') && name.trim() == name
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Directive {
    line: usize,
    action: Option<Action>,
    /// The listed patterns, each translated once when the directive is read; `None` for all rules.
    patterns: Option<Vec<Vec<Glob>>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Enable,
    Disable,
}

/// SQLFluff's directives in a file, by line.
#[derive(Clone, Debug, Default)]
pub(crate) struct SqlfluffIndex {
    directives: Vec<Directive>,
}

impl SqlfluffIndex {
    /// From comment tokens: each comment's 1-based start line and its full text (delimiters included).
    pub(crate) fn from_comments<'a>(comments: impl IntoIterator<Item = (usize, &'a str)>) -> Self {
        let mut directives = Vec::new();
        for (line, text) in comments {
            for (offset, piece) in text.split('\n').enumerate() {
                let mut comment = piece.trim();
                if comment.ends_with("*/") {
                    comment = comment[..comment.len() - 2].trim_end();
                }
                if comment.starts_with("/*") {
                    comment = comment[2..].trim_start();
                }
                if let Some((action, patterns)) = parse(comment) {
                    directives.push(Directive {
                        line: line + offset,
                        action,
                        patterns,
                    });
                }
            }
        }
        directives.sort_by_key(|directive| directive.line);
        Self { directives }
    }

    /// Reads every `comment` node of the file `root` belongs to (tree-sitter-sql names `--` and
    /// `/* */` comments `comment`), except those inside a routine's `AS $$ ... $$` body: the
    /// grammar parses that body as SQL, but SQLFluff's PostgreSQL dialect lexes it as one
    /// dollar-quoted string, so a comment there is no directive. A `BEGIN ATOMIC` body is SQL to
    /// both, and its comments count.
    pub(crate) fn read<D: Doc>(root: &Node<'_, D>, source: &str, lines: &LineTable) -> Self {
        Self::from_comments(root.dfs().filter_map(|node| {
            (node.kind() == "comment" && !in_dollar_quoted_body(&node)).then(|| {
                let range = node.range();
                (lines.line(range.start) + 1, &source[range])
            })
        }))
    }

    /// Whether a violation of the rule whose references are `references` (its code first), starting on 1-based `line`, is suppressed.
    pub(crate) fn suppresses(&self, references: &[String], line: usize) -> bool {
        let matches = |directive: &Directive| {
            directive.patterns.as_ref().is_none_or(|patterns| {
                patterns.iter().any(|globs| {
                    references
                        .iter()
                        .any(|reference| fnmatchcase(globs, reference))
                })
            })
        };
        if self.directives.iter().any(|directive| {
            directive.line == line && directive.action.is_none() && matches(directive)
        }) {
            return true;
        }
        let mut ignored = false;
        for directive in self.directives.iter().filter(|directive| {
            directive.action.is_some() && directive.line <= line && matches(directive)
        }) {
            ignored = directive.action == Some(Action::Disable);
        }
        ignored
    }
}

/// Whether `comment` lies between the opening and closing `dollar_quote` of an enclosing
/// `function_body` (`AS $tag$ ... $tag$`). A comment between `AS` and the opening tag is outside
/// the string, as SQLFluff lexes it.
fn in_dollar_quoted_body<D: Doc>(comment: &Node<'_, D>) -> bool {
    let range = comment.range();
    comment
        .ancestors()
        .filter(|node| node.kind() == "function_body")
        .any(|body| {
            let mut quotes = body
                .children()
                .filter(|child| child.kind() == "dollar_quote");
            match (quotes.next(), quotes.last()) {
                (Some(open), Some(close)) => {
                    open.range().end <= range.start && range.end <= close.range().start
                }
                _ => false,
            }
        })
}

fn parse(comment: &str) -> Option<(Option<Action>, Option<Vec<Vec<Glob>>>)> {
    let comment = comment.rsplit("--").next()?.trim();
    if !comment.starts_with("noqa") {
        return None;
    }
    let remainder = &comment[4..];
    if remainder.is_empty() {
        return Some((None, None));
    }
    if !remainder.starts_with(':') {
        return None;
    }
    let remainder = remainder[1..].trim();
    if remainder.is_empty() {
        return Some((None, None));
    }
    let (action, rule_part) = if let Some((action, rules)) = remainder.split_once('=') {
        let action = match action {
            "disable" => Action::Disable,
            "enable" => Action::Enable,
            _ => return None,
        };
        (Some(action), rules)
    } else {
        if matches!(remainder, "disable" | "enable") {
            return None;
        }
        (None, remainder)
    };
    if rule_part == "all" {
        return Some((action, None));
    }
    Some((
        action,
        Some(
            rule_part
                .split(',')
                .map(|rule| translate(rule.trim()))
                .collect(),
        ),
    ))
}

/// One step of a translated `fnmatch` pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Glob {
    /// `*`, consecutive stars collapsed.
    Star,
    /// One character: `?` (any), a literal, or a bracket set.
    One(Set),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Set {
    Any,
    Char(char),
    /// A bracket expression: its characters and inclusive ranges, negated with `!`.
    Class {
        negated: bool,
        chars: Vec<char>,
        ranges: Vec<(char, char)>,
    },
}

impl Set {
    fn contains(&self, value: char) -> bool {
        match self {
            Set::Any => true,
            Set::Char(c) => *c == value,
            Set::Class {
                negated,
                chars,
                ranges,
            } => {
                let hit = chars.contains(&value)
                    || ranges.iter().any(|&(lo, hi)| lo <= value && value <= hi);
                hit != *negated
            }
        }
    }
}

/// Python 3.12's `fnmatch.translate`, as steps: `*` (collapsed), `?`, `[...]` sets, literals.
/// Each `[` finds its closing `]` in a table built once, so the walk is linear in the pattern
/// (rescanning to the end for every unclosed `[` would be quadratic).
fn translate(pattern: &str) -> Vec<Glob> {
    let pattern: Vec<char> = pattern.chars().collect();
    // `close[j]`: the position of the first `]` at or after `j`, or the pattern's length.
    let mut close = vec![pattern.len(); pattern.len() + 1];
    for j in (0..pattern.len()).rev() {
        close[j] = if pattern[j] == ']' { j } else { close[j + 1] };
    }
    let mut globs = Vec::new();
    let mut i = 0;
    while i < pattern.len() {
        let c = pattern[i];
        i += 1;
        match c {
            '*' => {
                if globs.last() != Some(&Glob::Star) {
                    globs.push(Glob::Star);
                }
            }
            '?' => globs.push(Glob::One(Set::Any)),
            '[' => {
                let mut j = i;
                if pattern.get(j) == Some(&'!') {
                    j += 1;
                }
                if pattern.get(j) == Some(&']') {
                    j += 1;
                }
                let j = close[j];
                if j >= pattern.len() {
                    globs.push(Glob::One(Set::Char('[')));
                } else {
                    globs.push(Glob::One(class(&pattern[i..j])));
                    i = j + 1;
                }
            }
            c => globs.push(Glob::One(Set::Char(c))),
        }
    }
    globs
}

/// The set a bracket expression's body (between `[` and `]`) matches, as `translate` builds it:
/// the body splits into chunks at each hyphen that makes a range (not one right after the opening
/// or its `!`, nor one right after a range's end, nor one ending the body); a range between the
/// last character of one chunk and the first of the next is dropped, with both, when it is empty;
/// a `!` the result then starts with negates; every other character, hyphens included, is itself.
///
/// Whether a range is empty depends only on the two chunks it joins (a chunk between two ranges
/// has at least two characters, so merging never changes the characters a later range compares),
/// so one forward pass decides each range in turn where `translate` walks them backwards.
fn class(body: &[char]) -> Set {
    // The body with dropped ranges merged away; `range_starts` holds each kept range's low end.
    let mut items: Vec<char> = Vec::with_capacity(body.len());
    let mut range_starts = Vec::new();
    let last = body.len().saturating_sub(1);
    let mut start = 0;
    let mut k = if body.first() == Some(&'!') { 2 } else { 1 };
    loop {
        let hyphen = body
            .get(k..last)
            .and_then(|rest| rest.iter().position(|&c| c == '-'))
            .map(|offset| k + offset);
        let chunk = &body[start..hyphen.unwrap_or(body.len())];
        if start == 0 {
            items.extend_from_slice(chunk);
        } else if body[start - 2] > chunk[0] {
            items.pop();
            items.extend_from_slice(&chunk[1..]);
        } else {
            range_starts.push(items.len() - 1);
            items.extend_from_slice(chunk);
        }
        let Some(hyphen) = hyphen else {
            break;
        };
        start = hyphen + 1;
        k = hyphen + 3;
    }
    let negated = items.first() == Some(&'!');
    let mut range_starts = range_starts.into_iter().peekable();
    let mut chars = Vec::new();
    let mut ranges = Vec::new();
    let mut index = usize::from(negated);
    // A lone `!` before a range leaves `[^-x...]`: that hyphen and `x` are themselves.
    if negated && range_starts.next_if_eq(&0).is_some() {
        chars.push('-');
    }
    while index < items.len() {
        if range_starts.next_if_eq(&index).is_some() {
            ranges.push((items[index], items[index + 1]));
            index += 2;
        } else {
            chars.push(items[index]);
            index += 1;
        }
    }
    Set::Class {
        negated,
        chars,
        ranges,
    }
}

/// Python's `fnmatch.fnmatchcase`, without filesystem normalization: the translated pattern
/// `globs` matched over the whole text. Every step but `*` takes one character, so the classic
/// backtrack-to-the-last-star walk decides it in time linear in the pattern times the text,
/// without recursion.
fn fnmatchcase(globs: &[Glob], text: &str) -> bool {
    let text: Vec<char> = text.chars().collect();
    let (mut g, mut t) = (0, 0);
    // The step after the last star, and the text position that star resumes from.
    let mut resume: Option<(usize, usize)> = None;
    while t < text.len() {
        match globs.get(g) {
            Some(Glob::Star) => {
                g += 1;
                resume = Some((g, t));
            }
            Some(Glob::One(set)) if set.contains(text[t]) => {
                g += 1;
                t += 1;
            }
            _ => match resume {
                Some((after, from)) => {
                    g = after;
                    t = from + 1;
                    resume = Some((after, from + 1));
                }
                None => return false,
            },
        }
    }
    globs[g..].iter().all(|glob| *glob == Glob::Star)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::oracle;
    use serde_json::Value;

    fn refs(code: &str) -> Vec<String> {
        let values: &[&str] = match code {
            "CV05" => &[
                "CV05",
                "L049",
                "all",
                "convention",
                "convention.is_null",
                "core",
            ],
            "CV08" => &["CV08", "L055", "all", "convention", "convention.left_join"],
            _ => &[code],
        };
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn parses_directives_and_globs() {
        let index = SqlfluffIndex::from_comments([
            (1, "-- noqa"),
            (2, "-- noqa: x"),
            (3, "-- a reason -- noqa: CV05"),
            (4, "-- noqa: CV05 -- a reason"),
            (5, "-- noqa: disable"),
            (6, "-- noqa: enable=CV05"),
            (7, "-- noqa: CV*"),
            (8, "-- noqa: [CV]V0[58]"),
            (9, "/* noqa:\ndisable=CV05 */"),
        ]);
        assert!(index.suppresses(&refs("CV05"), 1));
        assert!(!index.suppresses(&refs("CV05"), 2));
        assert!(index.suppresses(&refs("CV05"), 3));
        assert!(!index.suppresses(&refs("CV05"), 4));
        assert!(!index.suppresses(&refs("CV05"), 5));
        assert!(!index.suppresses(&refs("CV05"), 6));
        assert!(index.suppresses(&refs("CV05"), 7));
        assert!(index.suppresses(&refs("CV05"), 8));
        assert!(index.suppresses(&refs("CV05"), 9));
        assert!(!index.suppresses(&refs("CV05"), 10));
    }

    /// Python 3.12's `fnmatch.fnmatchcase(text, pattern)` on each case, as the SQL tool venv's
    /// interpreter answers it.
    #[test]
    fn fnmatchcase_agrees_with_python() {
        let cases: &[(&str, &str, bool)] = &[
            ("CV*", "CV05", true),
            ("*", "convention.is_null", true),
            ("**", "x", true),
            ("*.*", "convention.is_null", true),
            ("*.is_*", "convention.is_null", true),
            ("*a*b", "ab", true),
            ("*a*b", "aab", true),
            ("*a*b", "abba", false),
            ("a*", "a", true),
            ("*z", "convention.is_null", false),
            ("c*n*l", "convention.is_null", true),
            ("?V05", "CV05", true),
            ("?", "", false),
            ("[CV]V0[58]", "CV05", true),
            ("[!C]V05", "CV05", false),
            ("[!X]V05", "CV05", true),
            ("[]]", "]", true),
            ("[]a]", "a", true),
            ("[!]]", "x", true),
            ("[!]]", "]", false),
            ("[!]", "!", false),
            ("[!]", "[", false),
            ("[!]x", "[!]x", true),
            ("[", "[", true),
            ("[a", "[a", true),
            ("a[", "a[", true),
            ("[a-c]", "b", true),
            ("[a-c]", "-", false),
            ("[c-a]", "b", false),
            ("[!c-a]", "b", true),
            ("[a-]", "-", true),
            ("[-a]", "-", true),
            ("[!-a]", "-", false),
            ("[!-a]", "b", true),
            ("[a-c-e]", "-", true),
            ("[a-c-e]", "d", false),
            ("[a-c-e]", "e", true),
            ("[z-a-q]", "-", true),
            ("[z-a-q]", "q", true),
            ("[z-a-q]", "b", false),
            ("[\\]", "\\", true),
            ("[^a]", "^", true),
            ("[^a]", "b", false),
            ("[[]", "[", true),
            ("[a&&b]", "&", true),
            ("[a|b]", "|", true),
            ("[~]", "~", true),
            ("*[0-9]", "CV05", true),
            ("CV0[!0-4]", "CV05", true),
            ("CV0[!0-5]", "CV05", false),
            ("[a-cx-z]", "y", true),
            ("[a-cx-z]", "-", false),
            ("[ab-]", "-", true),
            ("[!z-ab-c]", "b", false),
            ("[!z-ab-c]", "z", true),
            ("*?*", "a", true),
            ("*?*", "", false),
            ("a*?", "a", false),
            ("a*?", "ab", true),
            ("?*\n", "x\n", true),
            ("*", "", true),
            ("", "", true),
            ("", "a", false),
            ("a\\b", "a\\b", true),
            ("[.]*", ".", true),
            ("*********************Z", "convention.is_null", false),
            // An empty range dropped from the front leaves a `!` that negates.
            ("[z-a!]", "x", true),
            ("[z-a!]", "!", true),
            ("[z-a!b]", "b", false),
            ("[z-a!b]", "c", true),
            // A lone `!` left before a kept range reads `[^-c]`: the hyphen and `c` are themselves.
            ("[z-a!-c]", "c", false),
            ("[z-a!-c]", "-", false),
            ("[z-a!-c]", "b", true),
            ("[z-a!-c]", "!", true),
            // Merged ranges keep the hyphens and characters they no longer join.
            ("[c-b-a]", "-", true),
            ("[c-b-a]", "a", true),
            ("[c-b-a]", "b", false),
            ("[x-a-c-e]", "d", true),
            ("[x-a-c-e]", "-", true),
            ("[x-a-c-e]", "a", false),
            ("[]-a]", "^", true),
            ("[]-a]", "b", false),
        ];
        for &(pattern, text, expected) in cases {
            assert_eq!(
                fnmatchcase(&translate(pattern), text),
                expected,
                "{pattern:?} on {text:?}"
            );
        }
    }

    /// A `noqa` comment's stars cannot make matching exponential: each check is linear in the
    /// pattern times the reference.
    #[test]
    fn many_stars_match_in_bounded_time() {
        let started = std::time::Instant::now();
        let pattern = format!("{}Z", "*".repeat(200));
        let alternating = format!("{}Z", "a*".repeat(200));
        let index = SqlfluffIndex::from_comments([
            (1, format!("-- noqa: {pattern}").as_str()),
            (2, format!("-- noqa: {alternating}").as_str()),
        ]);
        assert!(!index.suppresses(&refs("CV05"), 1));
        assert!(!index.suppresses(&refs("CV05"), 2));
        let long = "a".repeat(400);
        assert!(!fnmatchcase(&translate(&alternating), &long));
        assert!(fnmatchcase(&translate(&alternating), &format!("{long}Z")));
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }

    /// A `noqa` pattern translates once per directive, in time linear in its length: a `[` with no
    /// closing `]` is not rescanned to the end, nor a bracket expression of many merged ranges.
    #[test]
    fn long_bracket_patterns_translate_in_linear_time() {
        let started = std::time::Instant::now();
        let unclosed = "[".repeat(100_000);
        let merged = format!("[{}ab]", "ab-".repeat(50_000));
        let index = SqlfluffIndex::from_comments([
            (1, format!("-- noqa: disable={unclosed}").as_str()),
            (2, format!("-- noqa: {merged}").as_str()),
        ]);
        for line in 1..=100 {
            assert!(!index.suppresses(&refs("CV05"), line));
        }
        assert!(fnmatchcase(&translate(&unclosed), &unclosed));
        let merged = translate(&merged);
        assert!(fnmatchcase(&merged, "a"));
        assert!(fnmatchcase(&merged, "b"));
        assert!(!fnmatchcase(&merged, "-"));
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }

    const ORACLE: &str = oracle!("sql/suppress/sqlfluff.expected.json");

    fn oracle_findings(value: &Value, file: &str, list: &str) -> Vec<(String, usize)> {
        value[file][list]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|finding| {
                let code = finding[0].as_str()?;
                if code != "CV05" && code != "CV08" {
                    return None;
                }
                Some((code.to_owned(), finding[1].as_u64()? as usize))
            })
            .collect()
    }

    #[test]
    fn matches_sqlfluff_oracle() {
        let oracle: Value = serde_json::from_str(ORACLE).unwrap();
        let comments = [
            (
                "inline.sql",
                vec![
                    (2, "-- noqa"),
                    (3, "-- noqa: CV05"),
                    (4, "-- noqa:CV05,CV08"),
                    (5, "-- noqa: CV05, CV08"),
                    (6, "--noqa: convention.is_null"),
                    (7, "-- noqa: L049"),
                    (8, "-- noqa: CV*"),
                    (9, "-- noqa: convention"),
                    (10, "-- noqa: core"),
                    (11, "-- noqa: all"),
                    (12, "-- noqa: cv05"),
                    (13, "-- NOQA"),
                    (14, "-- noqa:"),
                    (15, "-- a reason -- noqa: CV05"),
                    (16, "-- noqa: CV05 -- a reason"),
                    (17, "/* noqa */"),
                    (18, "/* noqa: CV05 */"),
                    (19, "/* noqa: CV05 */"),
                    (20, "-- noqa CV05"),
                    (21, "-- noqa: XX99"),
                    (22, "-- noqa: disable"),
                    (23, "-- noqa: PRS"),
                    (25, "-- noqa"),
                    (28, "-- noqa: CV05"),
                    (29, "-- noqa: CV05"),
                    (30, "-- noqa: CV05,"),
                ],
            ),
            (
                "postgres_bodies.sql",
                vec![
                    // Line 2's `-- noqa` is inside a `$$` body, one string to SQLFluff.
                    (4, "-- noqa: CV05"),
                    (8, "/* outer /* nested */ -- noqa */"),
                    (9, "/* noqa:\ndisable=CV05 */"),
                ],
            ),
            (
                "range.sql",
                vec![
                    (2, "-- noqa: disable=CV05"),
                    (4, "-- noqa: enable=CV05"),
                    (6, "-- noqa: disable=all"),
                    (8, "-- noqa: enable=all"),
                    (10, "-- noqa: disable=CV05,CV08"),
                    (12, "-- noqa: enable=CV08"),
                    (14, "-- noqa: enable=all"),
                    (15, "-- noqa: disable=CV05"),
                    (17, "/* noqa: enable=CV05 */"),
                    (18, "/* noqa: disable=CV05 */"),
                    (19, "/* noqa: enable=all */"),
                    (20, "-- noqa: disable=convention.is_null"),
                    (22, "-- noqa: enable=CV05"),
                    (23, "-- noqa: disable=CV*"),
                    (25, "-- noqa: enable=all"),
                    (26, "-- noqa: enable=CV08"),
                    (28, "-- noqa: disable=CV08"),
                ],
            ),
            (
                "routine_bodies.sql",
                vec![
                    // Lines 2 and 6 are inside `$$` bodies; line 10 is in a `BEGIN ATOMIC` body.
                    (10, "-- noqa: disable=CV08"),
                    (14, "-- noqa: disable=CV05"),
                ],
            ),
        ];
        for (file, comments) in comments {
            let index = SqlfluffIndex::from_comments(comments);
            let kept: std::collections::HashSet<_> =
                oracle_findings(&oracle, file, "kept").into_iter().collect();
            for (code, line) in oracle_findings(&oracle, file, "all") {
                assert_eq!(
                    index.suppresses(&refs(&code), line),
                    !kept.contains(&(code.clone(), line)),
                    "{file}:{line} {code}"
                );
            }
        }
    }

    #[test]
    fn read_reads_the_tree_like_the_hand_list() {
        use crate::code_rule_dsl::language::GrammarDialect;
        use crate::code_rule_dsl::lines::Terminators;
        use ast_grep_language::LanguageExt;
        let oracle: Value = serde_json::from_str(ORACLE).unwrap();
        for file in [
            "inline.sql",
            "postgres_bodies.sql",
            "range.sql",
            "routine_bodies.sql",
        ] {
            let path = std::path::Path::new(crate::code_rule_dsl::fixtures_dir!())
                .join("sql/suppress/sqlfluff")
                .join(file);
            let source = std::fs::read_to_string(path).unwrap();
            let grep = GrammarDialect::Sql.ast_grep(&source);
            let index = SqlfluffIndex::read(
                &grep.root(),
                &source,
                &LineTable::new(&source, Terminators::Newlines),
            );
            let kept: std::collections::HashSet<_> =
                oracle_findings(&oracle, file, "kept").into_iter().collect();
            let mut checked = 0;
            for (code, line) in oracle_findings(&oracle, file, "all") {
                assert_eq!(
                    index.suppresses(&refs(&code), line),
                    !kept.contains(&(code.clone(), line)),
                    "{file}:{line} {code}"
                );
                checked += 1;
            }
            assert!(checked > 0, "{file}");
        }
    }
}
