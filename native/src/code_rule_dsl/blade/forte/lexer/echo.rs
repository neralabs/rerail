//! `Lexer/Concerns/EchoScanner.php`.
use super::super::scan;
use super::super::token::{ErrorReason, State, TokenType};
use super::Lexer;
use super::construct::CONSTRUCT_ALL;

impl Lexer<'_> {
    /// The branches `scanEcho`, `scanRawEcho`, `scanTripleEcho` and `scanDirectiveArgs` share,
    /// in their order: past a PHP string, heredoc or comment starting at `byte`; `false` when
    /// `byte` starts none of them.
    pub(super) fn skip_php_literal(&mut self, byte: u8) -> bool {
        if byte == b'\'' || byte == b'"' {
            self.pos += 1;
            scan::skip_quoted_string(self.source, &mut self.pos, self.len, byte);
        } else if byte == b'`' {
            self.pos += 1;
            scan::skip_backtick_string(self.source, &mut self.pos, self.len);
        } else if byte == b'<'
            && self.peek_ahead(1) == Some(b'<')
            && self.peek_ahead(2) == Some(b'<')
        {
            self.pos += 3;
            scan::skip_heredoc(self.source, &mut self.pos, self.len);
        } else if byte == b'/' && self.peek_ahead(1) == Some(b'*') {
            self.pos += 2;
            scan::skip_block_comment(self.source, &mut self.pos, self.len);
        } else if byte == b'/' && self.peek_ahead(1) == Some(b'/') {
            self.pos += 2;
            self.skip_line_comment_with_warnings();
        } else if byte == b'#' {
            self.pos += 1;
            self.skip_line_comment_with_warnings();
        } else {
            return false;
        }
        true
    }

    /// The `{`/`@`/`<` branch of the echo scans: at a Blade construct, logs the collision, emits
    /// the content so far and returns to the return state (`true`); otherwise steps past.
    fn echo_collides(&mut self, state: State, content_start: usize) -> bool {
        if self.detect_construct(CONSTRUCT_ALL).is_some() {
            self.log_error(state, ErrorReason::ConstructCollision, self.pos);

            if content_start < self.pos {
                self.emit_token(TokenType::EchoContent, content_start, self.pos);
            }

            self.state = self.return_state;
            self.return_state = State::Data;

            return true;
        }

        self.pos += 1;
        false
    }

    /// The end of the echo scans at EOF.
    fn echo_eof(&mut self, state: State, content_start: usize) {
        self.log_error(state, ErrorReason::UnexpectedEof, self.len);
        if content_start < self.pos {
            self.emit_token(TokenType::EchoContent, content_start, self.pos);
        }
        self.pos = self.len; // Prevent duplicate tokens
        self.state = self.return_state;
        self.return_state = State::Data;
    }

    /// `scanEcho`: from `{{` (known to be there).
    pub(super) fn scan_echo(&mut self) {
        let start = self.pos;

        if self.peek() == Some(b'{')
            && self.peek_ahead(1) == Some(b'{')
            && self.peek_ahead(2) == Some(b'{')
        {
            self.scan_triple_echo();

            return;
        }

        self.emit_token(TokenType::EchoStart, start, start + 2);
        self.pos += 2;

        // Scan content until }}
        let content_start = self.pos;

        while self.pos < self.len {
            let byte = self.source[self.pos];

            if self.skip_php_literal(byte) {
            } else if byte == b'{' || byte == b'@' || byte == b'<' {
                if self.echo_collides(State::EchoContent, content_start) {
                    return;
                }
            } else if byte == b'}' && self.peek_ahead(1) == Some(b'}') {
                if content_start < self.pos {
                    self.emit_token(TokenType::EchoContent, content_start, self.pos);
                }
                self.emit_token(TokenType::EchoEnd, self.pos, self.pos + 2);
                self.pos += 2;
                self.state = self.return_state;
                self.return_state = State::Data;

                return;
            } else {
                self.pos += 1;
            }
        }

        self.echo_eof(State::EchoContent, content_start);
    }

    /// `scanRawEcho`: from `{!!` (known to be there).
    pub(super) fn scan_raw_echo(&mut self) {
        let start = self.pos;

        self.emit_token(TokenType::RawEchoStart, start, start + 3);
        self.pos += 3;

        let content_start = self.pos;

        while self.pos < self.len {
            let byte = self.source[self.pos];

            if self.skip_php_literal(byte) {
            } else if byte == b'{' || byte == b'@' || byte == b'<' {
                if self.echo_collides(State::RawEchoContent, content_start) {
                    return;
                }
            } else if byte == b'!'
                && self.peek_ahead(1) == Some(b'!')
                && self.peek_ahead(2) == Some(b'}')
            {
                if content_start < self.pos {
                    self.emit_token(TokenType::EchoContent, content_start, self.pos);
                }
                self.emit_token(TokenType::RawEchoEnd, self.pos, self.pos + 3);
                self.pos += 3;
                self.state = self.return_state;
                self.return_state = State::Data;

                return;
            } else {
                self.pos += 1;
            }
        }

        self.echo_eof(State::RawEchoContent, content_start);
    }

    /// `scanTripleEcho`: from `{{{` (known to be there).
    pub(super) fn scan_triple_echo(&mut self) {
        let start = self.pos;

        self.emit_token(TokenType::TripleEchoStart, start, start + 3);
        self.pos += 3;

        // Scan content until }}}
        let content_start = self.pos;

        while self.pos < self.len {
            let byte = self.source[self.pos];

            if self.skip_php_literal(byte) {
            } else if byte == b'{' || byte == b'@' || byte == b'<' {
                if self.echo_collides(State::TripleEchoContent, content_start) {
                    return;
                }
            } else if byte == b'}'
                && self.peek_ahead(1) == Some(b'}')
                && self.peek_ahead(2) == Some(b'}')
            {
                if content_start < self.pos {
                    self.emit_token(TokenType::EchoContent, content_start, self.pos);
                }
                self.emit_token(TokenType::TripleEchoEnd, self.pos, self.pos + 3);
                self.pos += 3;
                self.state = self.return_state;
                self.return_state = State::Data;

                return;
            } else {
                self.pos += 1;
            }
        }

        self.echo_eof(State::TripleEchoContent, content_start);
    }
}
