//! Ruff 0.16.9's `# noqa` comments (`crates/ruff_linter/src/noqa.rs`, `checkers/noqa.rs`) and the
//! line mapping they are read through (`directives.rs`'s `extract_noqa_line_for`).

use super::names;
use crate::code_rule_dsl::json::{invalid, object, reject_unknown};
use crate::code_rule_dsl::lines::LineTable;
use ast_grep_core::{Doc, Node};
use serde_json::Value;
use std::ops::Range;

/// `noqa`: Ruff's `# noqa` and file-level `# ruff: noqa` comments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Noqa {
    /// The codes a directive must name (or name none) to suppress the violation.
    pub(crate) codes: Vec<String>,
    /// `parent: "statement"`: a directive where the enclosing statement starts also suppresses
    /// it (Ruff's `set_parent`).
    pub(crate) parent: bool,
}

/// `suppress.noqa`.
pub(super) fn spec(noqa: &Value, at: &str) -> napi::Result<Noqa> {
    reject_unknown(object(noqa, at)?, &["codes", "parent"], at)?;
    let codes = names(
        noqa.get("codes"),
        &format!("{at}/codes"),
        rule_code,
        "rule codes",
        "a rule code (uppercase letters, then digits: F401)",
    )?;
    let parent = match noqa.get("parent") {
        None => false,
        Some(Value::String(parent)) if parent == "statement" => true,
        Some(_) => return Err(invalid(format!("{at}/parent must be \"statement\""))),
    };
    Ok(Noqa { codes, parent })
}

/// A code a `# noqa` list can name: Ruff's lexer reads uppercase letters, then digits.
fn rule_code(code: &str) -> bool {
    let letters = code.trim_start_matches(|c: char| c.is_ascii_uppercase());
    letters.len() < code.len() && !letters.is_empty() && letters.chars().all(|c| c.is_ascii_digit())
}

/// A directive: every rule, or the listed codes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Directive {
    All,
    Codes(Vec<String>),
}

impl Directive {
    fn includes(&self, codes: &[String]) -> bool {
        match self {
            Self::All => true,
            Self::Codes(listed) => listed.iter().any(|code| codes.contains(code)),
        }
    }
}

/// Why a directive suppresses nothing: Ruff's `LexicalError`.
#[derive(Debug, PartialEq, Eq)]
enum Refused {
    InvalidSuffix,
    MissingCodes,
    InvalidCodeSuffix,
}

/// Ruff's `is_noqa_uncased`.
fn noqa_uncased(text: &str) -> bool {
    text.len() >= 4 && text.as_bytes()[..4].eq_ignore_ascii_case(b"noqa")
}

/// A cursor over a comment's text.
struct Cursor<'a> {
    text: &'a str,
    at: usize,
}

impl<'a> Cursor<'a> {
    fn rest(&self) -> &'a str {
        &self.text[self.at..]
    }
    fn first(&self) -> Option<char> {
        self.rest().chars().next()
    }
    fn bump(&mut self) -> Option<char> {
        let c = self.first()?;
        self.at += c.len_utf8();
        Some(c)
    }
    fn eat_while(&mut self, keep: impl Fn(char) -> bool) {
        while let Some(c) = self.first().filter(|c| keep(*c)) {
            self.at += c.len_utf8();
        }
    }
    fn eat_char(&mut self, wanted: char) -> bool {
        if self.first() == Some(wanted) {
            self.at += wanted.len_utf8();
            true
        } else {
            false
        }
    }
    fn eat_whitespace(&mut self) {
        self.eat_while(char::is_whitespace);
    }
    fn is_eof(&self) -> bool {
        self.at >= self.text.len()
    }
    fn skip_to_end(&mut self) {
        self.at = self.text.len();
    }
}

/// `NoqaLexer::lex_directive`, from just after `noqa`.
fn lex_directive(cursor: &mut Cursor<'_>) -> Result<Directive, Refused> {
    match cursor.bump() {
        None | Some('#') => return Ok(Directive::All),
        Some(':') => {}
        Some(c) if c.is_whitespace() => {
            cursor.eat_whitespace();
            if cursor.bump() != Some(':') {
                return Ok(Directive::All);
            }
        }
        Some(_) => return Err(Refused::InvalidSuffix),
    }
    let mut codes = Vec::new();
    let mut missing_delimiter = false;
    while !cursor.is_eof() {
        lex_code(cursor, &mut codes, &mut missing_delimiter)?;
    }
    if codes.is_empty() {
        return Err(Refused::MissingCodes);
    }
    Ok(Directive::Codes(codes))
}

/// `NoqaLexer::lex_code`: one code, or the end of the list.
fn lex_code(
    cursor: &mut Cursor<'_>,
    codes: &mut Vec<String>,
    missing_delimiter: &mut bool,
) -> Result<(), Refused> {
    cursor.eat_whitespace();
    if cursor.first() == Some(',') {
        cursor.bump();
        return Ok(());
    }
    let start = cursor.at;
    match cursor.bump() {
        Some(c) if c.is_ascii_uppercase() => {
            cursor.eat_while(|c| c.is_ascii_uppercase());
            if !cursor.first().is_some_and(|c| c.is_ascii_digit()) {
                if *missing_delimiter {
                    return Err(Refused::InvalidCodeSuffix);
                }
                // The first word that is no code ends the list (a trailing comment).
                cursor.skip_to_end();
                return Ok(());
            }
            cursor.eat_while(|c| c.is_ascii_digit());
            codes.push(cursor.text[start..cursor.at].to_owned());
            if cursor.is_eof() {
                return Ok(());
            }
            *missing_delimiter = match cursor.first() {
                Some(',') => {
                    cursor.bump();
                    false
                }
                Some(c) if c.is_whitespace() => false,
                Some(c) if c.is_ascii_uppercase() => true,
                Some('#') => false,
                _ => return Err(Refused::InvalidCodeSuffix),
            };
        }
        Some(_) => cursor.skip_to_end(),
        None => {}
    }
    Ok(())
}

/// `lex_inline_noqa`: the first `# noqa` among a comment's `#` sections.
fn lex_inline(comment: &str) -> Option<Result<Directive, Refused>> {
    let mut cursor = Cursor {
        text: comment,
        at: 0,
    };
    while !cursor.is_eof() {
        cursor.eat_while(|c| c != '#');
        if !cursor.eat_char('#') {
            continue;
        }
        cursor.eat_whitespace();
        if !noqa_uncased(cursor.rest()) {
            continue;
        }
        cursor.at += "noqa".len();
        return Some(lex_directive(&mut cursor));
    }
    None
}

/// `lex_file_exemption`: `#\s*(?:ruff|flake8)\s*:\s*(?i)noqa` and its directive.
fn lex_file(comment: &str) -> Option<Result<Directive, Refused>> {
    let mut cursor = Cursor {
        text: comment,
        at: 0,
    };
    while !cursor.is_eof() {
        cursor.eat_while(|c| c != '#');
        if !cursor.eat_char('#') {
            continue;
        }
        cursor.eat_whitespace();
        if cursor.rest().starts_with("ruff") {
            cursor.at += "ruff".len();
        } else if cursor.rest().starts_with("flake8") {
            cursor.at += "flake8".len();
        } else {
            continue;
        }
        cursor.eat_whitespace();
        if !cursor.eat_char(':') {
            continue;
        }
        cursor.eat_whitespace();
        if !noqa_uncased(cursor.rest()) {
            continue;
        }
        cursor.at += "noqa".len();
        return Some(lex_directive(&mut cursor));
    }
    None
}

/// An inline directive and the line it stands on (`NoqaDirectiveLine`).
#[derive(Debug)]
struct DirectiveLine {
    /// The line without its terminator.
    line: Range<usize>,
    /// Whether the directive also applies at the line's end: the file's last line.
    includes_end: bool,
    directive: Directive,
}

/// A Python file's `noqa` comments and line mapping.
#[derive(Debug, Default)]
pub(crate) struct NoqaIndex {
    /// File-level exemptions: every rule, or these codes (`FileExemption`).
    file_all: bool,
    file_codes: Vec<String>,
    lines: Vec<DirectiveLine>,
    /// `NoqaMapping`: an offset inside a range reads the directive where the range ends.
    mapping: Vec<Range<usize>>,
}

/// The statements whose start is a violation's parent offset (Ruff's `set_parent`: an import
/// statement for F401 and F811, the `__all__` definition for F822).
const PARENT_KINDS: &[&str] = &[
    "import_statement",
    "import_from_statement",
    "future_import_statement",
    "expression_statement",
];

impl NoqaIndex {
    /// Reads the comments, strings and line continuations of the file `node` belongs to.
    pub(crate) fn read<D: Doc>(root: &Node<'_, D>, source: &str, lines: &LineTable) -> Self {
        let mut index = Self::default();
        let mut mapping = Vec::new();
        let mut continuations = Vec::new();
        let mut first_leaf = None;
        // Where the last leaf ended: a backslash before a line break in the gap to the next leaf
        // continues the line (Ruff's indexer finds newlines between tokens; tree-sitter-python
        // keeps no `line_continuation` node between the parts of an implicit concatenation).
        let mut last_end = None;
        for node in root.dfs() {
            if node.children().next().is_none() && !node.range().is_empty() {
                let range = node.range();
                first_leaf.get_or_insert(range.clone());
                if let Some(end) = last_end.filter(|end| *end < range.start) {
                    let gap = &source[end..range.start];
                    for (at, _) in gap.match_indices('\\') {
                        if gap[at + 1..].starts_with(['\n', '\r']) {
                            continuations.push(lines.line(end + at));
                        }
                    }
                }
                last_end = Some(range.end);
            }
            match node.kind().as_ref() {
                "comment" => {
                    index.comment(&source[node.range()], node.range().start, source, lines)
                }
                // The outermost strings: an f-string's nested ones lie inside its range.
                "string" if !node.ancestors().any(|ancestor| ancestor.kind() == "string") => {
                    let range = node.range();
                    let text = &source[range.clone()];
                    let prefix = text.split(['\'', '"']).next().unwrap_or_default();
                    let quotes = &text[prefix.len()..];
                    let interpolated = prefix.contains(['f', 'F', 't', 'T']);
                    let triple = quotes.starts_with("\"\"\"") || quotes.starts_with("'''");
                    if (interpolated || triple) && text.contains(['\n', '\r']) {
                        mapping.push(lines.start(lines.line(range.start))..range.end);
                    }
                }
                "line_continuation" => continuations.push(lines.line(node.range().start)),
                _ => {}
            }
        }
        // Runs of continued lines read the directive on the line after the last one.
        continuations.sort_unstable();
        continuations.dedup();
        let mut run: Option<Range<usize>> = None;
        for line in continuations {
            let (start, end) = (lines.start(line), lines.full_end(line));
            match run.as_mut() {
                Some(last) if last.end == start => last.end = end,
                _ => {
                    mapping.extend(run.replace(start..end));
                }
            }
        }
        mapping.extend(run);
        // A leading shebang reads the directive on the next line.
        if let Some(first) = first_leaf.filter(|first| {
            source[first.clone()].starts_with("#!")
                && source[..first.start]
                    .chars()
                    .all(|c| matches!(c, ' ' | '\t' | '\x0c'))
        }) {
            mapping.push(first.start..lines.full_end(lines.line(first.end)));
        }
        mapping.sort_by_key(|range| (range.start, range.end));
        for range in mapping {
            match index.mapping.last_mut() {
                Some(last) if last.end >= range.start => last.end = last.end.max(range.end),
                _ => index.mapping.push(range),
            }
        }
        index
    }

    fn comment(&mut self, text: &str, start: usize, source: &str, lines: &LineTable) {
        let line = lines.line(start);
        // A file exemption must stand on its own line.
        let own_line = source[lines.start(line)..start]
            .chars()
            .all(|c| matches!(c, ' ' | '\t' | '\x0c'));
        if own_line && let Some(Ok(directive)) = lex_file(text) {
            match directive {
                Directive::All => self.file_all = true,
                Directive::Codes(codes) => self.file_codes.extend(codes),
            }
        }
        if let Some(Ok(directive)) = lex_inline(text) {
            let range = lines.start(line)..lines.end(line);
            self.lines.push(DirectiveLine {
                includes_end: range.end == source.len(),
                line: range,
                directive,
            });
        }
    }

    /// `NoqaMapping::resolve`.
    fn resolve(&self, offset: usize) -> usize {
        self.mapping
            .iter()
            .find(|range| range.contains(&offset))
            .map_or(offset, |range| range.end)
    }

    /// Whether a diagnostic of one of `codes` starting at `at`, with the parent offset `parent`,
    /// is suppressed (`check_noqa`): by a file exemption, else by the directive on the line an
    /// offset resolves to.
    pub(crate) fn suppresses(&self, codes: &[String], at: usize, parent: Option<usize>) -> bool {
        if self.file_all || self.file_codes.iter().any(|code| codes.contains(code)) {
            return true;
        }
        let mut offsets = parent
            .into_iter()
            .chain([at])
            .map(|offset| self.resolve(offset))
            .collect::<Vec<_>>();
        offsets.dedup();
        offsets.into_iter().any(|offset| {
            self.lines
                .iter()
                .find(|line| {
                    line.line.start <= offset
                        && (offset < line.line.end
                            || (offset == line.line.end && line.includes_end))
                })
                .is_some_and(|line| line.directive.includes(codes))
        })
    }
}

/// Where the statement around a violation's start begins: the innermost import or expression
/// statement holding `at`. `node` is a node of the file, near `at`.
pub(crate) fn parent_statement<D: Doc>(node: &Node<'_, D>, at: usize) -> Option<usize> {
    let holds = |node: &Node<'_, D>| node.range().start <= at && at < node.range().end;
    let mut deepest = node.clone();
    while !holds(&deepest) {
        deepest = deepest.parent()?;
    }
    loop {
        let child = deepest.children().find(holds);
        match child {
            Some(child) => deepest = child,
            None => break,
        }
    }
    std::iter::once(deepest.clone())
        .chain(deepest.ancestors())
        .find(|node| PARENT_KINDS.contains(&node.kind().as_ref()))
        .map(|statement| statement.range().start)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inline(comment: &str) -> Option<Result<Directive, Refused>> {
        lex_inline(comment)
    }

    #[test]
    fn noqa_comments_lex_as_ruff_lexes_them() {
        let codes = |codes: &[&str]| {
            Some(Ok(Directive::Codes(
                codes.iter().map(|code| (*code).to_owned()).collect(),
            )))
        };
        assert_eq!(inline("# noqa"), Some(Ok(Directive::All)));
        assert_eq!(inline("#NOQA"), Some(Ok(Directive::All)));
        assert_eq!(inline("# noqa: F401,E501"), codes(&["F401", "E501"]));
        assert_eq!(inline("# noqa : F401"), codes(&["F401"]));
        assert_eq!(inline("# noqa a comment"), Some(Ok(Directive::All)));
        assert_eq!(inline("# noqa: F401 because"), codes(&["F401"]));
        assert_eq!(inline("# noqa:F401F841"), codes(&["F401", "F841"]));
        assert_eq!(inline("# noqa: F401,,E501"), codes(&["F401", "E501"]));
        assert_eq!(inline("# noqa: F401#more"), codes(&["F401"]));
        assert_eq!(inline("# a comment # noqa: F401"), codes(&["F401"]));
        assert_eq!(inline("#noqaF401"), Some(Err(Refused::InvalidSuffix)));
        assert_eq!(inline("# noqa:"), Some(Err(Refused::MissingCodes)));
        assert_eq!(inline("# noqa: f401"), Some(Err(Refused::MissingCodes)));
        assert_eq!(
            inline("# noqa: F401F"),
            Some(Err(Refused::InvalidCodeSuffix))
        );
        assert_eq!(
            inline("# noqa: F401-"),
            Some(Err(Refused::InvalidCodeSuffix))
        );
        assert_eq!(inline("# ruff: noqa"), None);
        assert_eq!(lex_file("# ruff: noqa"), Some(Ok(Directive::All)));
        assert_eq!(lex_file("#flake8:NOQA: F401"), codes(&["F401"]));
        assert_eq!(lex_file("# Ruff: noqa"), None);
        assert_eq!(lex_file("# noqa"), None);
    }
}
