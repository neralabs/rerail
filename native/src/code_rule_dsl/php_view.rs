//! The PHP view of a `.php` file: the bytes tree-sitter parses for it.
//!
//! `tree-sitter-php`'s `php_only` grammar, which PHP patterns, kind ids and Blade's embedded PHP
//! use, reads a whole file as PHP: inline text after a mid-file `?>` parses as code. The view
//! keeps what PHP's lexer reads as PHP and blanks the rest, byte for byte, so a tape encoded
//! against the original source keeps its spans, texts and lines:
//! - inline text outside PHP mode becomes spaces; blanks and line ends stay;
//! - the first open tag stays; later ones (`<?php`, `<?=`) become spaces;
//! - a mid-file `?>` becomes PHP's implicit `;` where a statement needs it (`terminates`), else
//!   spaces; a final `?>`, followed only by inline text, stays;
//! - a line comment a mid-file `?>` ends becomes a block comment of the same bytes.
//!
//! A file with nothing to blank (pure PHP, at most a final `?>` before blanks) is its own view.

use super::blade::forte::scan;
use super::language::GrammarDialect;
use ast_grep_core::AstGrep;
use ast_grep_core::tree_sitter::StrDoc;
use ast_grep_language::LanguageExt;
use std::borrow::Cow;
use std::ops::Range;

/// What tree-sitter parses for `source` in `language`: a PHP file's view, else the source.
pub(super) fn parse_input(language: GrammarDialect, source: &str) -> Cow<'_, str> {
    if language == GrammarDialect::Php {
        view(source)
    } else {
        Cow::Borrowed(source)
    }
}

/// `language`'s ast-grep tree of `parse_input`, reading `source`'s text.
pub(super) fn ast_grep(language: GrammarDialect, source: &str) -> AstGrep<StrDoc<GrammarDialect>> {
    match parse_input(language, source) {
        Cow::Borrowed(_) => language.ast_grep(source),
        Cow::Owned(input) => {
            let mut doc = StrDoc::new(&input, language);
            // The view has `source`'s length and character boundaries, so every node's range
            // reads `source`.
            doc.src = source.to_owned();
            AstGrep::doc(doc)
        }
    }
}

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
    let mut pos = first.end;
    // The last byte of PHP code read, outside blanks and comments (`"` ends a string or heredoc).
    let mut last = None;
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
                    last = Some(b';');
                }
                pos = open.end;
            }
            b'\'' | b'"' | b'`' => {
                pos += 1;
                scan::skip_quoted_string(bytes, &mut pos, len, byte);
                last = Some(b'"');
            }
            b'/' if next == Some(b'/') => {
                let start = pos;
                pos += 2;
                if scan::skip_line_comment_stopping_at(bytes, &mut pos, len, b"?>") {
                    comment = Some(start);
                }
            }
            b'/' if next == Some(b'*') => {
                pos += 2;
                scan::skip_block_comment(bytes, &mut pos, len);
            }
            // `#[` opens a PHP 8 attribute, not a comment.
            b'#' if next != Some(b'[') => {
                let start = pos;
                pos += 1;
                if scan::skip_line_comment_stopping_at(bytes, &mut pos, len, b"?>") {
                    comment = Some(start);
                }
            }
            b'<' if bytes[pos..].starts_with(b"<<<") => {
                pos += 3;
                skip_heredoc(bytes, &mut pos);
                last = Some(b'"');
            }
            b' ' | b'\t' | b'\n' | b'\r' => pos += 1,
            _ => {
                last = Some(byte);
                pos += 1;
            }
        }
    }
    view.finish()
}

/// Whether a mid-file `?>` keeps PHP's implicit `;` in the view. Before any code, or after `;`,
/// `:` (an alternative-syntax or `case` body) or `}`, it would be an empty statement. After `{`
/// it stays, so a block holding only inline text is not empty (PHP_CodeSniffer's
/// `EmptyStatement` reads tags and inline text as content), unless a `case` or `default` follows:
/// a switch body holds no statement.
fn terminates(last: Option<u8>, source: &[u8], resume: usize) -> bool {
    match last {
        None | Some(b';' | b':' | b'}') => false,
        Some(b'{') => !opens_case(source, resume),
        Some(_) => true,
    }
}

/// Whether the PHP code at `at` starts with `case` or `default`.
fn opens_case(source: &[u8], mut at: usize) -> bool {
    while at < source.len() && matches!(source[at], b' ' | b'\t' | b'\n' | b'\r') {
        at += 1;
    }
    let start = at;
    while at < source.len() && is_label(source[at]) {
        at += 1;
    }
    let word = &source[start..at];
    word.eq_ignore_ascii_case(b"case") || word.eq_ignore_ascii_case(b"default")
}

/// The first open tag at or after `from`, as Forte's `phpTagStartLength` reads one: `<?php` (in
/// any case) not followed by a letter, digit or `_`, or `<?=`. A bare `<?` is no tag.
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

/// A byte of a PHP label: a letter, digit, `_` or any byte of a multi-byte character.
fn is_label(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}

/// From after `<<<`: past a heredoc's or nowdoc's closing label, as PHP 7.3+ reads one: the
/// opening label may follow blanks and be quoted; the closing label may be indented and is
/// followed by any byte that cannot continue it. (Forte's `scan::skip_heredoc` closes only at an
/// unindented label before `;` or a line end, so it would read past an indented closing label.)
/// Without a label, `<<<` is no heredoc and `pos` stays after it.
fn skip_heredoc(source: &[u8], pos: &mut usize) {
    let len = source.len();
    let mut at = *pos;
    while at < len && matches!(source[at], b' ' | b'\t') {
        at += 1;
    }
    let quote = match source.get(at) {
        Some(&quote @ (b'\'' | b'"')) => {
            at += 1;
            Some(quote)
        }
        _ => None,
    };
    let start = at;
    while at < len && is_label(source[at]) {
        at += 1;
    }
    if at == start || source[start].is_ascii_digit() {
        return;
    }
    let label = &source[start..at];
    if let Some(quote) = quote {
        if source.get(at) != Some(&quote) {
            return;
        }
        at += 1;
    }
    *pos = at;
    loop {
        let Some(end) = scan::find_line_ending(source, *pos, Some(len)) else {
            *pos = len;
            return;
        };
        *pos = end;
        scan::skip_line_ending(source, pos, len);
        let mut at = *pos;
        while at < len && matches!(source[at], b' ' | b'\t') {
            at += 1;
        }
        if source[at..].starts_with(label)
            && source
                .get(at + label.len())
                .is_none_or(|&byte| !is_label(byte))
        {
            *pos = at + label.len();
            return;
        }
    }
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
