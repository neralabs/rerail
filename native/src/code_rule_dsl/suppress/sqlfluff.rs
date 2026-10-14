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
    patterns: Option<Vec<String>>,
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

    /// Reads every `comment` node of the file `root` belongs to (tree-sitter-sql names `--` and `/* */` comments `comment`).
    pub(crate) fn read<D: Doc>(root: &Node<'_, D>, source: &str, lines: &LineTable) -> Self {
        Self::from_comments(root.dfs().filter_map(|node| {
            (node.kind() == "comment").then(|| {
                let range = node.range();
                (lines.line(range.start) + 1, &source[range])
            })
        }))
    }

    /// Whether a violation of the rule whose references are `references` (its code first), starting on 1-based `line`, is suppressed.
    pub(crate) fn suppresses(&self, references: &[String], line: usize) -> bool {
        let matches = |directive: &Directive| {
            directive.patterns.as_ref().is_none_or(|patterns| {
                patterns.iter().any(|pattern| {
                    references
                        .iter()
                        .any(|reference| fnmatchcase(pattern, reference))
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

fn parse(comment: &str) -> Option<(Option<Action>, Option<Vec<String>>)> {
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
                .map(|rule| rule.trim().to_owned())
                .collect(),
        ),
    ))
}

/// One step of a translated `fnmatch` pattern.
#[derive(Debug, PartialEq, Eq)]
enum Glob {
    /// `*`, consecutive stars collapsed.
    Star,
    /// One character: `?` (any), a literal, or a bracket set.
    One(Set),
}

#[derive(Debug, PartialEq, Eq)]
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
fn translate(pattern: &[char]) -> Vec<Glob> {
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
                while j < pattern.len() && pattern[j] != ']' {
                    j += 1;
                }
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
/// or its `!`, nor one right after a range's end); a range between the last character of one chunk
/// and the first of the next is dropped, with both, when it is empty; a `!` the result then starts
/// with negates; every other character, hyphens included, is itself.
fn class(body: &[char]) -> Set {
    let mut chunks: Vec<Vec<char>> = Vec::new();
    if body.contains(&'-') {
        let mut start = 0;
        let mut k = if body.first() == Some(&'!') { 2 } else { 1 };
        while let Some(offset) = body
            .get(k..)
            .and_then(|rest| rest.iter().position(|&c| c == '-'))
        {
            let hyphen = k + offset;
            chunks.push(body[start..hyphen].to_vec());
            start = hyphen + 1;
            k = hyphen + 3;
        }
        if start < body.len() {
            chunks.push(body[start..].to_vec());
        } else if let Some(last) = chunks.last_mut() {
            last.push('-');
        }
        for k in (1..chunks.len()).rev() {
            let (Some(&end), Some(&first)) = (chunks[k - 1].last(), chunks[k].first()) else {
                continue;
            };
            if end > first {
                let next = chunks.remove(k);
                let previous = &mut chunks[k - 1];
                previous.pop();
                previous.extend_from_slice(&next[1..]);
            }
        }
    } else {
        chunks.push(body.to_vec());
    }
    let negated = chunks[0].first() == Some(&'!');
    if negated {
        chunks[0].remove(0);
    }
    let mut chars = Vec::new();
    let mut ranges = Vec::new();
    let count = chunks.len();
    for (index, chunk) in chunks.iter().enumerate() {
        let from = usize::from(index > 0);
        let to = if index + 1 < count {
            chunk.len().saturating_sub(1)
        } else {
            chunk.len()
        };
        chars.extend(chunk.get(from..to.max(from)).unwrap_or_default());
        let next = chunks.get(index + 1).and_then(|next| next.first());
        if let (Some(&lo), Some(&hi)) = (chunk.last(), next) {
            ranges.push((lo, hi));
        }
    }
    Set::Class {
        negated,
        chars,
        ranges,
    }
}

/// Python's `fnmatch.fnmatchcase`, without filesystem normalization: the translated pattern
/// matched over the whole text. Every step but `*` takes one character, so the classic
/// backtrack-to-the-last-star walk decides it in time linear in the pattern times the text,
/// without recursion.
fn fnmatchcase(pattern: &str, text: &str) -> bool {
    let globs = translate(&pattern.chars().collect::<Vec<_>>());
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
        ];
        for &(pattern, text, expected) in cases {
            assert_eq!(
                fnmatchcase(pattern, text),
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
        assert!(!fnmatchcase(&alternating, &long));
        assert!(fnmatchcase(&alternating, &format!("{long}Z")));
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
                    (2, "-- noqa"),
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
        for file in ["inline.sql", "postgres_bodies.sql", "range.sql"] {
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
                // Line 2 of postgres_bodies.sql is inside a `$$` body the grammar parses as SQL; SQLFluff reports nothing there.
                if file == "postgres_bodies.sql" && line == 2 {
                    continue;
                }
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
