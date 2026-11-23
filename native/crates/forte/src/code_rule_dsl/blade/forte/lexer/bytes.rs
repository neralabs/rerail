//! `Lexer/Concerns/ByteHelpers.php`.
use super::super::scan::is_space;
use super::super::token::TokenType;
use super::Lexer;

impl Lexer<'_> {
    /// PHP's `$this->source[$at]`: `None` past the end, where PHP reads `""`.
    #[inline]
    pub(super) fn at(&self, at: usize) -> Option<u8> {
        if at < self.len {
            Some(self.source[at])
        } else {
            None
        }
    }

    /// `peek`.
    #[inline]
    pub(super) fn peek(&self) -> Option<u8> {
        self.at(self.pos)
    }

    /// `peekAhead`.
    #[inline]
    pub(super) fn peek_ahead(&self, n: usize) -> Option<u8> {
        self.at(self.pos + n)
    }

    /// `skipWhitespace`.
    pub(super) fn skip_whitespace(&mut self) {
        while self.pos < self.len && is_space(self.source[self.pos]) {
            self.pos += 1;
        }
    }

    /// `skipAndEmitWhitespace`.
    pub(super) fn skip_and_emit_whitespace(&mut self) {
        let start = self.pos;

        while self.pos < self.len && is_space(self.source[self.pos]) {
            self.pos += 1;
        }

        if start < self.pos {
            self.emit_token(TokenType::Whitespace, start, self.pos);
        }
    }
}
