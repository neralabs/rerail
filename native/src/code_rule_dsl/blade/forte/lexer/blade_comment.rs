//! `Lexer/Concerns/BladeCommentScanner.php`.
use super::super::scan::strpos;
use super::super::token::{ErrorReason, State, TokenType};
use super::Lexer;

impl Lexer<'_> {
    /// `scanBladeCommentStart`: from `{{--` (known to be there).
    pub(super) fn scan_blade_comment_start(&mut self) {
        let start = self.pos;

        // Consume {{--
        self.pos += 4;

        self.emit_token(TokenType::BladeCommentStart, start, self.pos);

        self.state = State::BladeComment;
    }

    /// `scanBladeCommentContent`.
    pub(super) fn scan_blade_comment_content(&mut self) {
        let start = self.pos;

        if self.pos < self.len {
            // Find the next occurrence of --}}
            let Some(close_pos) = strpos(self.source, b"--}}", self.pos) else {
                // Hit EOF without finding --}}
                self.log_error(State::BladeComment, ErrorReason::UnclosedComment, self.len);

                if start < self.len {
                    self.emit_token(TokenType::Text, start, self.len);
                }

                self.pos = self.len;

                self.state = self.return_state;
                self.return_state = State::Data;

                return;
            };

            // Found the closing --}}
            let content_end = close_pos;

            if content_end > start {
                self.emit_token(TokenType::Text, start, content_end);
            }

            let end_start = content_end;
            self.pos = close_pos + 4;
            self.emit_token(TokenType::BladeCommentEnd, end_start, self.pos);

            self.state = self.return_state;
            self.return_state = State::Data;

            return;
        }

        // Fail safe.
        self.log_error(State::BladeComment, ErrorReason::UnclosedComment, self.len);

        if start < self.len {
            self.emit_token(TokenType::Text, start, self.len);
        }

        self.pos = self.len;
        self.state = self.return_state;
        self.return_state = State::Data;
    }
}
