//! The PHP view of a `.php` file: the bytes tree-sitter parses for it.
//!
//! `tree-sitter-php`'s `php_only` grammar, which PHP patterns, kind ids and Blade's embedded PHP
//! use, reads a whole file as PHP: inline text after a mid-file `?>` parses as code. The view
//! keeps what PHP's lexer reads as PHP and blanks the rest, byte for byte, so a tape encoded
//! against the original source keeps its spans, texts and lines:
//! - inline text outside PHP mode becomes spaces; blanks and line ends stay;
//! - the first open tag stays; later ones (`<?php`, `<?`) become spaces, and `<?=` before a blank
//!   becomes `echo`, as PHP compiles it (else spaces, its expression a statement);
//! - a mid-file `?>` becomes PHP's implicit `;` where a statement needs it (`terminates`), else
//!   spaces; a final `?>`, followed only by inline text, stays;
//! - a line comment a mid-file `?>` ends becomes a block comment of the same bytes.
//!
//! A file with nothing to blank (pure PHP, at most a final `?>` before blanks) is its own view.
//! PHP's string literals, heredocs and comments are read as `php_literal` reads them, so a `?>`
//! inside one, or inside a string an interpolation nests, stays PHP.

use super::blade::forte::scan;
use super::php_literal;
use std::borrow::Cow;
use std::ops::Range;

/// The PHP view of `source` (the module's rules); `source` itself where nothing is blanked.
pub(super) fn view(source: &str) -> Cow<'_, str> {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut view = Masked {
        source,
        bytes: None,
    };
    let Some(first) = open_tag(bytes, 0) else {
        view.blank(0..len);
        return view.finish();
    };
    view.blank(0..first.start);
    // The last PHP the view read before a `?>`, outside blanks and comments.
    let mut last = None;
    // The first tag is the grammar's `php_tag`, but for a `<?=` whose expressions only `echo`
    // reads as a statement.
    if bytes[first.clone()] == *b"<?=" && short_echo(&mut view, bytes, &first) {
        last = Some(Last::Code(b'o'));
    }
    let mut pos = first.end;
    // The line comment a `?>` ended, as PHP's lexer ends one.
    let mut comment = None;
    while pos < len {
        let byte = bytes[pos];
        let next = bytes.get(pos + 1).copied();
        match byte {
            b'?' if next == Some(b'>') => {
                let Some(open) = open_tag(bytes, pos + 2) else {
                    view.blank(pos + 2..len);
                    break;
                };
                // Without its `?>`, the grammar's line comment would run on to the line's end:
                // the view closes it as a block comment of its bytes, or blanks a shorter one.
                if let Some(start) = comment.take() {
                    view.blank(start..pos);
                    if pos - start >= 4 {
                        view.put(start, b"/*");
                        view.put(pos - 2, b"*/");
                    }
                }
                view.blank(pos..open.end);
                if terminates(last, bytes, open.end) {
                    view.put(pos, b";");
                    last = Some(Last::Code(b';'));
                }
                if bytes[open.clone()] == *b"<?=" && short_echo(&mut view, bytes, &open) {
                    last = Some(Last::Code(b'o'));
                }
                pos = open.end;
            }
            b'/' if next == Some(b'/') => {
                let start = pos;
                pos += 2;
                if scan::skip_line_comment_stopping_at(bytes, &mut pos, len, b"?>") {
                    comment = Some(start);
                }
            }
            // `#[` opens a PHP 8 attribute, not a comment.
            b'#' if next != Some(b'[') => {
                let start = pos;
                pos += 1;
                if scan::skip_line_comment_stopping_at(bytes, &mut pos, len, b"?>") {
                    comment = Some(start);
                }
            }
            b' ' | b'\t' | b'\n' | b'\r' => pos += 1,
            _ => match php_literal::literal_end(bytes, pos, 0) {
                // A block comment leaves `last` as it was; a string, heredoc or nowdoc is a
                // literal however its interpolations nest.
                Some(end) => {
                    if byte != b'/' {
                        last = Some(Last::Literal);
                    }
                    pos = end;
                }
                None => {
                    last = Some(Last::Code(byte));
                    pos += 1;
                }
            },
        }
    }
    view.finish()
}

/// What a mid-file `?>` follows in the PHP before it, which decides its implicit `;`.
#[derive(Clone, Copy)]
enum Last {
    /// A byte of code, outside literals and comments.
    Code(u8),
    /// A string literal, heredoc or nowdoc.
    Literal,
}

/// Whether a mid-file `?>` keeps PHP's implicit `;` in the view. Before any code, or after `;` or
/// `:` (an alternative-syntax or `case` body), it would be an empty statement. After `{` it
/// stays, so a block holding only inline text is not empty (PHP_CodeSniffer's `EmptyStatement`
/// reads tags and inline text as content), unless a `case` or `default` follows: a switch body
/// holds no statement. After `}` it stays too, since the brace may end an expression (a closure,
/// a `match`, `$o->{$name}`), unless what follows continues the statement the brace closes
/// (`else`, `catch`, a `do`'s `while`, ...).
fn terminates(last: Option<Last>, source: &[u8], resume: usize) -> bool {
    let next = next_word(source, resume);
    let follows = |words: &[&[u8]]| words.iter().any(|word| next.eq_ignore_ascii_case(word));
    match last {
        None | Some(Last::Code(b';' | b':')) => false,
        Some(Last::Code(b'{')) => !follows(&[b"case", b"default"]),
        Some(Last::Code(b'}')) => !follows(&[
            b"case", b"default", b"else", b"elseif", b"catch", b"finally", b"while",
        ]),
        Some(_) => true,
    }
}

/// The PHP word at `at` past blanks: empty where none starts there.
fn next_word(source: &[u8], mut at: usize) -> &[u8] {
    while at < source.len() && matches!(source[at], b' ' | b'\t' | b'\n' | b'\r') {
        at += 1;
    }
    let length = source[at..]
        .iter()
        .take_while(|byte| php_literal::label_byte(**byte))
        .count();
    &source[at..at + length]
}

/// The first open tag at or after `from`, as PHP reads one with `short_open_tag` on (its compiled
/// default, and what tree-sitter-php's `php_tag` reads): `<?php` (in any case) not followed by a
/// letter, digit or `_`, `<?=`, or `<?` before a blank (not `<?xml`).
fn open_tag(source: &[u8], mut from: usize) -> Option<Range<usize>> {
    while let Some(at) = scan::strpos(source, b"<?", from) {
        let rest = &source[at + 2..];
        let length = if rest.len() >= 3 && rest[..3].eq_ignore_ascii_case(b"php") {
            if rest
                .get(3)
                .is_some_and(|&byte| scan::is_alnum(byte) || byte == b'_')
            {
                0
            } else {
                5
            }
        } else if rest.first() == Some(&b'=') {
            3
        } else if rest
            .first()
            .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r'))
        {
            2
        } else {
            0
        };
        if length > 0 {
            return Some(at..at + length);
        }
        from = at + 2;
    }
    None
}

/// Writes a `<?=` tag as `echo` over it and the blank that follows (whether it did), so its
/// expressions read as the `echo` PHP compiles it to; else blanks it, its expression a statement.
fn short_echo(view: &mut Masked<'_>, source: &[u8], tag: &Range<usize>) -> bool {
    let echoes = matches!(source.get(tag.end), Some(b' ' | b'\t'));
    if echoes {
        view.put(tag.start, b"echo");
    } else {
        view.blank(tag.clone());
    }
    echoes
}

/// The view's bytes: `source`'s until one differs.
struct Masked<'a> {
    source: &'a str,
    bytes: Option<Vec<u8>>,
}

impl<'a> Masked<'a> {
    fn set(&mut self, at: usize, byte: u8) {
        match &mut self.bytes {
            Some(bytes) => bytes[at] = byte,
            None if self.source.as_bytes()[at] != byte => {
                let mut bytes = self.source.as_bytes().to_vec();
                bytes[at] = byte;
                self.bytes = Some(bytes);
            }
            None => {}
        }
    }

    /// `range` as spaces, keeping spaces, tabs and line ends.
    fn blank(&mut self, range: Range<usize>) {
        for at in range {
            if !matches!(self.source.as_bytes()[at], b' ' | b'\t' | b'\n' | b'\r') {
                self.set(at, b' ');
            }
        }
    }

    fn put(&mut self, at: usize, bytes: &[u8]) {
        for (offset, byte) in bytes.iter().enumerate() {
            self.set(at + offset, *byte);
        }
    }

    fn finish(self) -> Cow<'a, str> {
        match self.bytes {
            None => Cow::Borrowed(self.source),
            // Only whole characters are replaced, each byte by an ASCII one: every blanked range
            // starts and ends at an ASCII byte (a tag, `?>`) or at the source's ends.
            Some(bytes) => Cow::Owned(String::from_utf8(bytes).expect("the view is UTF-8")),
        }
    }
}

#[cfg(test)]
mod tests;
