//! PHPStan 2.2.16's local `@phpstan-ignore` comments.
//!
//! This reproduces `RichParser::getLinesToIgnore` over PHP's tokens: comments are tree-sitter's
//! `comment` nodes (PHP's `T_COMMENT` and `T_DOC_COMMENT`), so text in strings, heredocs, shell
//! commands or attributes is never a directive. Rerail's grammar reads a whole file as PHP, so the
//! text before the first open tag and after each `?>` is found as PHP's lexer finds it: one
//! `T_INLINE_HTML` token up to the next `<?php` or `<?=` (short tags off, as php.ini ships them).
//! A `-line` or `-next-line` ignore covers every error on its line; an ordinary
//! `@phpstan-ignore` lists identifiers for the line of the token before it, or else for the line
//! of the token after it. Then, as `LocalIgnoresProcessor::process`, each identifier listed for a
//! line ignores one error there: the first error with that identifier spends it.

use super::psalm::{doc_comment, pcre_space};
use super::{names, php_comment_end};
use crate::code_rule_dsl::json::{object, reject_unknown};
use crate::code_rule_dsl::lines::LineTable;
use ast_grep_core::{Doc, Node};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;

/// `phpstan.identifiers`: PHPStan diagnostic identifiers a local ignore must name.
pub(super) fn spec(phpstan: &Value, at: &str) -> napi::Result<Vec<String>> {
    reject_unknown(object(phpstan, at)?, &["identifiers"], at)?;
    names(
        phpstan.get("identifiers"),
        &format!("{at}/identifiers"),
        phpstan_identifier,
        "PHPStan identifiers",
        "a PHPStan identifier (letters or digits, with internal dots)",
    )
}

const IGNORE: &str = "@phpstan-ignore";
const IGNORE_LINE: &str = "@phpstan-ignore-line";
const IGNORE_NEXT_LINE: &str = "@phpstan-ignore-next-line";

#[derive(Debug, Default)]
pub(crate) struct PhpstanIndex {
    /// `$linesToIgnore`, by zero-based line: `None` ignores every error on the line; `Some` holds
    /// the identifiers ordinary ignores list for it, in order.
    lines: HashMap<usize, Option<Vec<String>>>,
    /// Per kind of error (a violation's code), the identifiers a line has left: copied from `lines`
    /// at the line's first error, less each identifier an error has spent.
    unspent: RefCell<HashMap<String, HashMap<usize, Vec<String>>>>,
}

/// An ordinary ignore with no token before it on its line, placed by the next token.
struct Pending<'s> {
    /// The comment's text after `@phpstan-ignore`.
    list: &'s str,
    /// The comment's last line.
    line: usize,
}

impl PhpstanIndex {
    pub(crate) fn read<D: Doc>(root: &Node<'_, D>, source: &str, lines: &LineTable) -> Self {
        let mut index = Self::default();
        // The first line of the last token that is neither white space nor a comment.
        let mut previous = None;
        let mut pending: Option<Pending<'_>> = None;
        // The end of the token being passed: tree-sitter splits a string, and may split an open
        // tag, into nodes.
        let mut within = 0;
        // Where the inline HTML being passed starts, and the open tag that ends it.
        let mut html = Some(0);
        let mut open = open_tag(source, 0);
        for node in root.dfs() {
            let range = node.range();
            if let Some(from) = html {
                let Some(tag) = open.clone().filter(|tag| range.start >= tag.start) else {
                    continue;
                };
                if from < tag.start {
                    index.token(lines.line(from), &mut previous, &mut pending);
                }
                index.token(lines.line(tag.start), &mut previous, &mut pending);
                html = None;
                within = tag.end;
            }
            if range.start < within {
                continue;
            }
            if node.kind() == "comment" {
                let text = &source[range.start..php_comment_end(source, &range)];
                index.comment(text, lines.line(range.start), previous, &mut pending);
                continue;
            }
            if constant_string(&node) {
                within = range.end;
            } else if !node.is_leaf() || range.is_empty() {
                continue;
            }
            index.token(lines.line(range.start), &mut previous, &mut pending);
            if matches!(&*node.kind(), "?>" | "php_end_tag") {
                // `T_CLOSE_TAG` takes one line break after it.
                let rest = &source[range.end..];
                let newline = if rest.starts_with("\r\n") {
                    2
                } else {
                    usize::from(rest.starts_with(['\r', '\n']))
                };
                let from = range.end + newline;
                html = Some(from);
                open = open_tag(source, from);
            }
        }
        if let Some(from) = html {
            if from < source.len() {
                index.token(lines.line(from), &mut previous, &mut pending);
            }
            if let Some(tag) = open {
                index.token(lines.line(tag.start), &mut previous, &mut pending);
            }
        }
        if let Some(Pending { list, line }) = pending
            && let Some(identifiers) = identifiers(list)
        {
            index.add(line, identifiers);
        }
        index
    }

    /// A token that is neither white space nor a comment, starting on `line`: it places a
    /// pending ignore, then becomes the previous token.
    fn token(
        &mut self,
        line: usize,
        previous: &mut Option<usize>,
        pending: &mut Option<Pending<'_>>,
    ) {
        if let Some(Pending { list, line: last }) = pending.take() {
            // PHPStan skips the rest of its loop for the token that meets a malformed list, so
            // that token never becomes the previous one.
            let Some(identifiers) = identifiers(list) else {
                return;
            };
            self.add(if line == last + 1 { line } else { last }, identifiers);
        }
        *previous = Some(line);
    }

    /// Whether the file holds no comment PHPStan would apply.
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// One comment token starting on `line`, after a token starting on `previous`.
    fn comment<'s>(
        &mut self,
        text: &'s str,
        line: usize,
        previous: Option<usize>,
        pending: &mut Option<Pending<'s>>,
    ) {
        let next = text.contains(IGNORE_NEXT_LINE);
        let current = text.contains(IGNORE_LINE);
        if doc_comment(text) {
            // PHP's `+=`: a line already listed keeps its entry.
            for at in positions(text, IGNORE_LINE) {
                self.lines.entry(line + breaks(&text[..at])).or_insert(None);
            }
            if next {
                let mut text = text;
                // A doc comment whose last tag is `-next-line` ignores the line after it, and its
                // text before that tag is read for more.
                if let Some(at) = last_tag(text.as_bytes())
                    && next_line_tag(text.as_bytes(), at)
                {
                    self.lines.insert(line + 1 + breaks(text), None);
                    text = &text[..at];
                }
                for at in positions(text, IGNORE_NEXT_LINE) {
                    self.lines
                        .entry(line + breaks(&text[..at]) + 1)
                        .or_insert(None);
                }
            }
            if next || current {
                return;
            }
        } else if next || current {
            self.lines
                .insert(line + usize::from(next) + breaks(text), None);
            return;
        }
        let Some(at) = text.find(IGNORE) else {
            return;
        };
        let list = &text[at + IGNORE.len()..];
        if previous == Some(line) {
            if let Some(identifiers) = identifiers(list) {
                self.add(line, identifiers);
            }
            return;
        }
        *pending = Some(Pending {
            list,
            line: line + breaks(text),
        });
    }

    /// PHP's `$lines[$line][] = $identifier`: a line that ignored every error now lists them.
    fn add(&mut self, line: usize, identifiers: Vec<String>) {
        self.lines
            .entry(line)
            .or_insert(None)
            .get_or_insert_with(Vec::new)
            .extend(identifiers);
    }

    /// Whether an error of `kind` with one of `identifiers` at byte `at` is ignored. Asked in
    /// source order, a kind's errors on one line spend the identifiers listed for it; another
    /// kind's errors spend their own copy.
    pub(crate) fn suppresses(
        &self,
        kind: &str,
        identifiers: &[String],
        at: usize,
        lines: &LineTable,
    ) -> bool {
        let line = lines.line(at);
        let listed = match self.lines.get(&line) {
            None => return false,
            Some(None) => return true,
            Some(Some(listed)) => listed,
        };
        let mut unspent = self.unspent.borrow_mut();
        if !unspent.contains_key(kind) {
            unspent.insert(kind.to_owned(), HashMap::new());
        }
        let left = unspent
            .get_mut(kind)
            .expect("inserted above")
            .entry(line)
            .or_insert_with(|| listed.clone());
        let Some(spent) = left.iter().position(|id| identifiers.contains(id)) else {
            return false;
        };
        left.remove(spent);
        true
    }
}

/// The open tag that ends inline HTML starting at `from`: PHP's lexer reads `<?php` (any case)
/// with the white space or line break after it, or at the end of the file, and `<?=`.
fn open_tag(source: &str, from: usize) -> Option<Range<usize>> {
    source[from..].match_indices("<?").find_map(|(at, _)| {
        let start = from + at;
        let rest = &source.as_bytes()[start + 2..];
        if rest.starts_with(b"=") {
            return Some(start..start + 3);
        }
        let php = rest.get(..3)?.eq_ignore_ascii_case(b"php");
        let after = match rest.get(3..) {
            Some(after) if after.starts_with(b"\r\n") => 2,
            Some([b' ' | b'\t' | b'\r' | b'\n', ..]) => 1,
            Some([]) => 0,
            _ => return None,
        };
        php.then_some(start..start + 5 + after)
    })
}

/// PHP's `T_CONSTANT_ENCAPSED_STRING`: a quoted string that interpolates nothing, one token that
/// starts on its first line.
fn constant_string<D: Doc>(node: &Node<'_, D>) -> bool {
    match &*node.kind() {
        "string" => node.is_named(),
        "encapsed_string" => node.children().all(|child| {
            !child.is_named() || matches!(&*child.kind(), "string_content" | "escape_sequence")
        }),
        _ => false,
    }
}

fn positions(text: &str, needle: &str) -> Vec<usize> {
    text.match_indices(needle).map(|(at, _)| at).collect()
}

/// PHPStan's `substr_count($text, "\n")`.
fn breaks(text: &str) -> usize {
    text.bytes().filter(|byte| *byte == b'\n').count()
}

/// Where the last match of `RichParser`'s caseless tag pattern starts:
/// `(@(?:[a-z][a-z0-9-\\]+:)?[a-z][a-z0-9-\\]*+)|(@[a-z_\\][a-z0-9_\:\\]*[a-z_][a-z0-9_]*)`.
/// A match holds no `@` but its first, so each `@` starts an attempt.
fn last_tag(text: &[u8]) -> Option<usize> {
    (0..text.len()).rev().find(|&at| {
        text[at] == b'@'
            && text.get(at + 1).is_some_and(|&first| {
                first.is_ascii_alphabetic()
                    || (matches!(first, b'_' | b'\\')
                        && text[at + 2..]
                            .iter()
                            .take_while(|byte| {
                                byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b':' | b'\\')
                            })
                            .any(|byte| byte.is_ascii_alphabetic() || *byte == b'_'))
            })
    })
}

/// Whether the tag match at `at` is exactly `@phpstan-ignore-next-line`: the pattern goes on
/// through `[a-z0-9-\\]`, and through a `:` that a letter follows.
fn next_line_tag(text: &[u8], at: usize) -> bool {
    let end = at + IGNORE_NEXT_LINE.len();
    text[at..].starts_with(IGNORE_NEXT_LINE.as_bytes())
        && !text
            .get(end)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'\\'))
        && !(text.get(end) == Some(&b':') && text.get(end + 1).is_some_and(u8::is_ascii_alphabetic))
}

/// `IgnoreLexer`'s token types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Token {
    Whitespace,
    End,
    Identifier,
    Comma,
    Open,
    Close,
    Other,
}

/// `IgnoreLexer::tokenize`'s anchored match at the start of `text`: the first of its patterns
/// that matches, and the match's length. `None` ends the tokens (a lone `\r`, say).
fn token(text: &[u8]) -> Option<(Token, usize)> {
    let run = |from: usize, keep: fn(u8) -> bool| {
        from + text[from..].iter().take_while(|byte| keep(**byte)).count()
    };
    let first = *text.first()?;
    Some(match first {
        b'\t' | b' ' => (
            Token::Whitespace,
            run(0, |byte| matches!(byte, b'\t' | b' ')),
        ),
        // `\r?+\n[\x09\x20]*+(?:\*(?!/)\x20?+)?` or `\*/`.
        b'\n' | b'\r' => {
            let newline = match (first, text.get(1)) {
                (b'\n', _) => 1,
                (_, Some(b'\n')) => 2,
                _ => return None,
            };
            let mut end = run(newline, |byte| matches!(byte, b'\t' | b' '));
            if text.get(end) == Some(&b'*') && text.get(end + 1) != Some(&b'/') {
                end += 1 + usize::from(text.get(end + 1) == Some(&b' '));
            }
            (Token::End, end)
        }
        b'*' if text.get(1) == Some(&b'/') => (Token::End, 2),
        // `Error::PATTERN_IDENTIFIER`: letters, digits and dots, ending on a letter or digit.
        _ if first.is_ascii_alphanumeric() => {
            let dotted = run(0, |byte| byte.is_ascii_alphanumeric() || byte == b'.');
            let end = text[..dotted]
                .iter()
                .rposition(u8::is_ascii_alphanumeric)
                .map_or(1, |at| at + 1);
            (Token::Identifier, end)
        }
        b',' => (Token::Comma, 1),
        b'(' => (Token::Open, 1),
        b')' => (Token::Close, 1),
        _ if pcre_space(char::from(first)) => return None,
        _ => (
            Token::Other,
            run(0, |byte| {
                !pcre_space(char::from(byte)) && !matches!(byte, b'(' | b')')
            }),
        ),
    })
}

/// `RichParser::parseIdentifiers` over a comment's text after `@phpstan-ignore`: identifiers
/// separated by commas, each with an optional parenthesised comment (nested parentheses
/// included), up to a line end; `None` where PHPStan reports a parse error instead.
fn identifiers(list: &str) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    let mut rest = list;
    while let Some((kind, length)) = token(rest.as_bytes()) {
        // Every token ends before an ASCII byte or at the end of the text.
        let (text, after) = rest.split_at_checked(length)?;
        tokens.push((kind, text));
        rest = after;
    }
    if tokens.last().is_none_or(|(kind, _)| *kind != Token::End) {
        tokens.push((Token::End, ""));
    }
    let mut identifiers = Vec::new();
    let mut expected: Option<&[Token]> = Some(&[Token::Identifier]);
    let mut open = 0usize;
    for (kind, text) in tokens {
        if let Some(expected) = expected
            && kind != Token::Whitespace
            && !expected.contains(&kind)
        {
            return None;
        }
        match kind {
            Token::Open => {
                open += 1;
                expected = None;
            }
            // Only expected inside parentheses, where `open` is positive.
            Token::Close => {
                open -= 1;
                if open == 0 {
                    expected = Some(&[Token::Comma, Token::End]);
                }
            }
            _ if open > 0 => {}
            Token::Identifier => {
                identifiers.push(text.to_owned());
                expected = Some(&[Token::Comma, Token::End, Token::Open]);
            }
            Token::Comma => expected = Some(&[Token::Identifier]),
            Token::Whitespace | Token::End | Token::Other => {}
        }
    }
    (open == 0 && !identifiers.is_empty()).then_some(identifiers)
}

pub(crate) fn phpstan_identifier(identifier: &str) -> bool {
    let bytes = identifier.as_bytes();
    !bytes.is_empty()
        && bytes[0].is_ascii_alphanumeric()
        && bytes.last().is_some_and(u8::is_ascii_alphanumeric)
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'.')
}

#[cfg(test)]
mod tests {
    use super::PhpstanIndex;
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::lines::{LineTable, Terminators};
    use crate::code_rule_dsl::oracle;
    use ast_grep_core::tree_sitter::LanguageExt;
    use serde_json::Value;

    const ORACLE: &str = oracle!("php/suppress/phpstan.expected.json");

    fn reports(file: &Value, key: &str) -> Vec<(String, usize)> {
        file[key]
            .as_array()
            .expect("reports")
            .iter()
            .map(|report| {
                (
                    report[0].as_str().expect("identifier").to_owned(),
                    report[1].as_u64().expect("start") as usize,
                )
            })
            .collect()
    }

    #[test]
    fn phpstan_comments_suppress_as_phpstan_honours_them() {
        let oracle: Value = serde_json::from_str(ORACLE).expect("oracle JSON");
        assert_eq!(
            oracle["phpstan"],
            "PHPStan - PHP Static Analysis Tool 2.2.16"
        );
        for (name, source) in [
            ("ignores.php", oracle!("php/suppress/phpstan/ignores.php")),
            (
                "terminators.php",
                oracle!("php/suppress/phpstan/terminators.php"),
            ),
            ("tokens.php", oracle!("php/suppress/phpstan/tokens.php")),
        ] {
            let root = GrammarDialect::Php.ast_grep(source);
            let lines = LineTable::new(source, Terminators::Newlines);
            let index = PhpstanIndex::read(&root.root(), source, &lines);
            // One PHPStan run's errors are one kind: they spend the same listed identifiers.
            let ours = reports(&oracle[name], "all")
                .into_iter()
                .filter(|(identifier, at)| {
                    !index.suppresses("phpstan", std::slice::from_ref(identifier), *at, &lines)
                })
                .collect::<Vec<_>>();
            // `ignore.*` reports are PHPStan's about the comments themselves.
            let kept = reports(&oracle[name], "kept")
                .into_iter()
                .filter(|(identifier, _)| !identifier.starts_with("ignore."))
                .collect::<Vec<_>>();
            assert_eq!(ours, kept, "{name}");
        }
    }
}
