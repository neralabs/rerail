//! SQLFluff 4.4.0's `noqa` directives, following `sqlfluff/core/rules/noqa.py`.

use crate::code_rule_dsl::lines::LineTable;
use ast_grep_core::{Doc, Node};

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
                    directives.push(Directive { line: line + offset, action, patterns });
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
                patterns.iter().any(|pattern| references.iter().any(|reference| fnmatchcase(pattern, reference)))
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
    Some((action, Some(rule_part.split(',').map(|rule| rule.trim().to_owned()).collect())))
}

/// Python's `fnmatch.fnmatchcase`, without filesystem normalization.
fn fnmatchcase(pattern: &str, text: &str) -> bool {
    fn matches(pattern: &[char], text: &[char]) -> bool {
        match pattern.first() {
            None => text.is_empty(),
            Some('*') => matches(&pattern[1..], text) || text.first().is_some_and(|_| matches(pattern, &text[1..])),
            Some('?') => text.first().is_some_and(|_| matches(&pattern[1..], &text[1..])),
            Some('[') => {
                let Some(end) = pattern.iter().position(|c| *c == ']') else {
                    return text.first() == Some(&'[') && matches(&pattern[1..], text.get(1..).unwrap_or_default());
                };
                let mut i = 1;
                let negated = pattern.get(i) == Some(&'!');
                if negated { i += 1; }
                let mut hit = false;
                let value = match text.first() { Some(value) => *value, None => return false };
                while i < end {
                    if i + 2 < end && pattern[i + 1] == '-' {
                        hit |= pattern[i] <= value && value <= pattern[i + 2];
                        i += 3;
                    } else {
                        hit |= pattern[i] == value;
                        i += 1;
                    }
                }
                (hit != negated) && matches(&pattern[end + 1..], &text[1..])
            }
            Some(ch) => text.first() == Some(ch) && matches(&pattern[1..], &text[1..]),
        }
    }
    matches(&pattern.chars().collect::<Vec<_>>(), &text.chars().collect::<Vec<_>>())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::oracle;
    use serde_json::Value;

    fn refs(code: &str) -> Vec<String> {
        let values: &[&str] = match code {
            "CV05" => &["CV05", "L049", "all", "convention", "convention.is_null", "core"],
            "CV08" => &["CV08", "L055", "all", "convention", "convention.left_join"],
            _ => &[code],
        };
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn parses_directives_and_globs() {
        let index = SqlfluffIndex::from_comments([
            (1, "-- noqa"), (2, "-- noqa: x"), (3, "-- a reason -- noqa: CV05"),
            (4, "-- noqa: CV05 -- a reason"), (5, "-- noqa: disable"),
            (6, "-- noqa: enable=CV05"), (7, "-- noqa: CV*"), (8, "-- noqa: [CV]V0[58]"),
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
        assert!(index.suppresses(&refs("CV05"), 10));
    }

    const ORACLE: &str = oracle!("sql/suppress/sqlfluff.expected.json");

    fn oracle_findings(value: &Value, file: &str, list: &str) -> Vec<(String, usize)> {
        value[file][list].as_array().unwrap().iter().filter_map(|finding| {
            let code = finding[0].as_str()?;
            if code != "CV05" && code != "CV08" { return None; }
            Some((code.to_owned(), finding[1].as_u64()? as usize))
        }).collect()
    }

    #[test]
    fn matches_sqlfluff_oracle() {
        let oracle: Value = serde_json::from_str(ORACLE).unwrap();
        let comments = [
            ("inline.sql", vec![(2,"-- noqa"),(3,"-- noqa: CV05"),(4,"-- noqa:CV05,CV08"),(5,"-- noqa: CV05, CV08"),(6,"--noqa: convention.is_null"),(7,"-- noqa: L049"),(8,"-- noqa: CV*"),(9,"-- noqa: convention"),(10,"-- noqa: core"),(11,"-- noqa: all"),(12,"-- noqa: cv05"),(13,"-- NOQA"),(14,"-- noqa:"),(15,"-- a reason -- noqa: CV05"),(16,"-- noqa: CV05 -- a reason"),(17,"/* noqa */"),(18,"/* noqa: CV05 */"),(19,"/* noqa: CV05 */"),(20,"-- noqa CV05"),(21,"-- noqa: XX99"),(22,"-- noqa: disable"),(23,"-- noqa: PRS"),(25,"-- noqa"),(28,"-- noqa: CV05"),(29,"-- noqa: CV05"),(30,"-- noqa: CV05,")]),
            ("postgres_bodies.sql", vec![(2,"-- noqa"),(4,"-- noqa: CV05"),(8,"/* outer /* nested */ -- noqa */"),(9,"/* noqa:\ndisable=CV05 */")]),
            ("range.sql", vec![(2,"-- noqa: disable=CV05"),(4,"-- noqa: enable=CV05"),(6,"-- noqa: disable=all"),(8,"-- noqa: enable=all"),(10,"-- noqa: disable=CV05,CV08"),(12,"-- noqa: enable=CV08"),(14,"-- noqa: enable=all"),(15,"-- noqa: disable=CV05"),(17,"/* noqa: enable=CV05 */"),(18,"/* noqa: disable=CV05 */"),(19,"/* noqa: enable=all */"),(20,"-- noqa: disable=convention.is_null"),(22,"-- noqa: enable=CV05"),(23,"-- noqa: disable=CV*"),(25,"-- noqa: enable=all"),(26,"-- noqa: enable=CV08"),(28,"-- noqa: disable=CV08")]),
        ];
        for (file, comments) in comments {
            let index = SqlfluffIndex::from_comments(comments);
            let kept: std::collections::HashSet<_> = oracle_findings(&oracle, file, "kept").into_iter().collect();
            for (code, line) in oracle_findings(&oracle, file, "all") {
                assert_eq!(index.suppresses(&refs(&code), line), !kept.contains(&(code.clone(), line)), "{file}:{line} {code}");
            }
        }
    }
}
