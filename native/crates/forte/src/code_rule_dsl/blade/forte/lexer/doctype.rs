//! `Lexer/Concerns/DoctypeScanner.php`.
use super::super::token::TokenType;
use super::Lexer;

impl Lexer<'_> {
    /// `scanDoctype`: from `<!DOCTYPE` (its nine bytes are known to be there).
    pub(super) fn scan_doctype(&mut self) {
        self.emit_token(TokenType::DoctypeStart, self.pos, self.pos + 9);
        self.pos += 9;

        self.skip_and_emit_whitespace();

        let content_start = self.pos;
        while self.pos < self.len && self.source[self.pos] != b'>' {
            self.pos += 1;
        }

        if content_start < self.pos {
            self.emit_token(TokenType::Doctype, content_start, self.pos);
        }

        if self.pos < self.len && self.source[self.pos] == b'>' {
            self.emit_token(TokenType::DoctypeEnd, self.pos, self.pos + 1);
            self.pos += 1;
        }
    }
}
