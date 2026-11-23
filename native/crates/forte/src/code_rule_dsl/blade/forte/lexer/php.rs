//! `Lexer/Concerns/PhpScanner.php`.
use super::super::scan::{self, is_alnum, is_alpha};
use super::super::token::TokenType;
use super::Lexer;

impl Lexer<'_> {
    /// `tryScanPhpTag`: `<?php` or `<?=` and its content up to `?>`.
    pub(super) fn try_scan_php_tag(&mut self) -> bool {
        if self.pos + 1 >= self.len
            || self.source[self.pos] != b'<'
            || self.source[self.pos + 1] != b'?'
        {
            return false;
        }

        let start = self.pos;
        self.pos += 2; // Skip <?

        if self.pos + 3 <= self.len {
            let matches_php = self.source[self.pos..self.pos + 3].eq_ignore_ascii_case(b"php");

            if matches_php {
                // Make sure it's followed by non-alphanumeric
                if let Some(next) = self.at(self.pos + 3)
                    && (is_alnum(next) || next == b'_')
                {
                    // Could be <?phps or something else, not a PHP tag
                    self.pos = start;

                    return false;
                }
                self.pos += 3;
                // Skip optional whitespace after <?php
                while self.pos < self.len
                    && matches!(self.source[self.pos], b' ' | b'\t' | b'\n' | b'\r')
                {
                    self.pos += 1;
                }
            } else if self.at(self.pos) == Some(b'=') {
                self.pos += 1; // <?=
            } else {
                // Just <?. Short tags are not supported and become a bogus comment later.
                self.pos = start;

                return false;
            }
        } else if self.at(self.pos) == Some(b'=') {
            self.pos += 1; // <?=
        } else {
            // Just <?.
            self.pos = start;

            return false;
        }

        self.emit_token(TokenType::PhpTagStart, start, self.pos);

        self.scan_php_content();

        true
    }

    /// `scanPhpContent`: up to and past `?>`, or up to a Blade directive.
    fn scan_php_content(&mut self) {
        let start = self.pos;

        loop {
            if self.pos >= self.len {
                // EOF
                if self.pos > start {
                    self.emit_token(TokenType::PhpContent, start, self.pos);
                }

                return;
            }

            // Check for ?> closing tag.
            if self.pos + 1 < self.len
                && self.source[self.pos] == b'?'
                && self.source[self.pos + 1] == b'>'
            {
                if self.pos > start {
                    self.emit_token(TokenType::PhpContent, start, self.pos);
                }
                self.emit_token(TokenType::PhpTagEnd, self.pos, self.pos + 2);
                self.pos += 2;

                return;
            }

            // Check for @ directive
            if self.source[self.pos] == b'@' && self.at(self.pos + 1).is_some_and(is_alpha) {
                // This is a Blade directive, stop PHP content here.
                if self.pos > start {
                    self.emit_token(TokenType::PhpContent, start, self.pos);
                }

                return;
            }

            let byte = self.source[self.pos];

            if byte == b'"' || byte == b'\'' {
                self.pos += 1;
                scan::skip_quoted_string(self.source, &mut self.pos, self.len, byte);
            } else if byte == b'/' && self.pos + 1 < self.len {
                let next = self.source[self.pos + 1];
                if next == b'/' {
                    self.pos += 2;
                    scan::skip_line_comment_stopping_at(
                        self.source,
                        &mut self.pos,
                        self.len,
                        b"?>",
                    );
                } else if next == b'*' {
                    self.pos += 2;
                    scan::skip_block_comment(self.source, &mut self.pos, self.len);
                } else {
                    self.pos += 1;
                }
            } else if byte == b'#' {
                self.pos += 1;
                scan::skip_line_comment_stopping_at(self.source, &mut self.pos, self.len, b"?>");
            } else if byte == b'<'
                && self.pos + 2 < self.len
                && self.source[self.pos + 1] == b'<'
                && self.source[self.pos + 2] == b'<'
            {
                // Heredoc/Nowdoc
                self.pos += 3;
                scan::skip_heredoc(self.source, &mut self.pos, self.len);
            } else {
                self.pos += 1;
            }
        }
    }

    /// `phpTagStartLength`: 5 for `<?php`, 3 for `<?=`, else 0.
    pub(super) fn php_tag_start_length(&self, pos: usize) -> usize {
        if pos + 2 > self.len || self.source[pos] != b'<' || self.source[pos + 1] != b'?' {
            return 0;
        }

        if pos + 5 <= self.len && self.source[pos + 2..pos + 5].eq_ignore_ascii_case(b"php") {
            if let Some(next) = self.at(pos + 5)
                && (is_alnum(next) || next == b'_')
            {
                // Could be <?phps or something else, not a PHP tag
                return 0;
            }

            return 5;
        }

        if pos + 3 <= self.len && self.source[pos + 2] == b'=' {
            return 3;
        }

        // Just <?. We do not support PHP short tags.
        0
    }

    /// `isPhpTagEndAt`.
    pub(super) fn is_php_tag_end_at(&self, pos: usize) -> bool {
        pos + 2 <= self.len && self.source[pos] == b'?' && self.source[pos + 1] == b'>'
    }
}
