//! `Lexer/Concerns/DirectiveScanner.php`.
use super::super::scan::{is_alnum, is_space};
use super::super::token::{ErrorReason, State, TokenType};
use super::Lexer;

impl Lexer<'_> {
    /// `scanDirective`: from `@`.
    pub(super) fn scan_directive(&mut self) {
        let start = self.pos;

        self.pos += 1;

        let name_start = self.pos;
        let mut pos = name_start;

        while pos < self.len {
            let byte = self.source[pos];
            if is_alnum(byte) || byte == b'_' {
                pos += 1;
            } else {
                break;
            }
        }

        if pos == name_start {
            self.emit_token(TokenType::Text, start, start + 1);
            self.state = self.return_state;
            self.return_state = State::Data;

            return;
        }

        let name = &self.source[name_start..pos];

        if !self.directives.is_directive(name) {
            // Not a known directive, emit everything scanned as text
            self.pos = pos;
            self.emit_token(TokenType::Text, start, pos);
            self.state = self.return_state;
            self.return_state = State::Data;

            return;
        }

        let mut arg_pos = pos;

        // Skip whitespace before potential args
        while arg_pos < self.len && is_space(self.source[arg_pos]) {
            arg_pos += 1;
        }

        let has_args = arg_pos < self.len && self.source[arg_pos] == b'(';

        self.pos = pos;
        if name.eq_ignore_ascii_case(b"php") && !has_args {
            self.emit_token(TokenType::PhpBlockStart, start, pos);
            self.set_php_block(true);
            self.state = self.return_state;
            self.return_state = State::Data;

            return;
        }

        if name.eq_ignore_ascii_case(b"endphp") {
            self.emit_token(TokenType::PhpBlockEnd, start, pos);
            if self.php_block {
                self.set_php_block(false);
            }
            self.state = self.return_state;
            self.return_state = State::Data;

            return;
        }

        if name.eq_ignore_ascii_case(b"verbatim") {
            self.emit_token(TokenType::VerbatimStart, start, pos);
            self.verbatim_return_state = Some(self.state);
            self.set_verbatim(true);
            self.state = State::Data;
            self.return_state = State::Data;

            return;
        }

        if name.eq_ignore_ascii_case(b"endverbatim") {
            self.emit_token(TokenType::VerbatimEnd, start, pos);
            self.state = self.verbatim_return_state.take().unwrap_or(State::Data);
            self.set_verbatim(false);
            self.return_state = State::Data;

            return;
        }

        self.emit_token(TokenType::Directive, start, pos);

        if has_args && arg_pos > pos {
            self.emit_token(TokenType::Whitespace, pos, arg_pos);
            self.pos = arg_pos;
        }

        if has_args {
            self.scan_directive_args();
        }

        self.state = self.return_state;
        self.return_state = State::Data;
    }

    /// `scanDirectiveArgs`: from `(`.
    fn scan_directive_args(&mut self) {
        let start = self.pos;

        if self.pos >= self.len || self.source[self.pos] != b'(' {
            return;
        }

        self.pos += 1;
        let mut depth = 1usize;

        while self.pos < self.len && depth > 0 {
            let byte = self.source[self.pos];

            if self.skip_php_literal(byte) {
            } else if byte == b'(' {
                depth += 1;
                self.pos += 1;
            } else if byte == b')' {
                depth -= 1;
                self.pos += 1;
            } else {
                self.pos += 1;
            }
        }

        if depth > 0 {
            self.log_error(State::Data, ErrorReason::UnexpectedEof, self.pos);
        }

        self.emit_token(TokenType::DirectiveArgs, start, self.pos);
    }

    /// `isEndverbatimAt`.
    pub(super) fn is_endverbatim_at(&self, pos: usize) -> bool {
        if pos + 12 > self.len || self.source[pos] != b'@' {
            return false;
        }

        if !self.source[pos + 1..pos + 12].eq_ignore_ascii_case(b"endverbatim") {
            return false;
        }

        if let Some(next) = self.at(pos + 12)
            && (is_alnum(next) || next == b'_')
        {
            return false;
        }

        true
    }

    /// `isEndphpAt`.
    pub(super) fn is_endphp_at(&self, pos: usize) -> bool {
        if pos + 7 > self.len || self.source[pos] != b'@' {
            return false;
        }

        if !self.source[pos + 1..pos + 7].eq_ignore_ascii_case(b"endphp") {
            return false;
        }

        if let Some(next) = self.at(pos + 7)
            && (is_alnum(next) || next == b'_')
        {
            return false;
        }

        true
    }
}
