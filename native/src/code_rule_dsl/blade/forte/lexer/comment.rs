//! `Lexer/Concerns/CommentScanner.php`.
use super::super::Exhausted;
use super::super::scan::{is_alnum, is_alpha, is_space, strpos};
use super::super::token::{ErrorReason, State, TokenType};
use super::Lexer;

impl Lexer<'_> {
    /// `scanComment`: from `<!--` (known to be there).
    pub(super) fn scan_comment(&mut self) {
        let start = self.pos;

        self.emit_token(TokenType::CommentStart, start, start + 4);
        self.pos += 4; // Skip <!--

        let content_start = self.pos;

        if self.pos < self.len {
            let Some(close_pos) = strpos(self.source, b"-->", self.pos) else {
                // EOF. Unclosed comment.
                if content_start < self.len {
                    self.emit_token(TokenType::Text, content_start, self.len);
                }
                self.log_error(State::Comment, ErrorReason::UnexpectedEof, self.len);
                self.pos = self.len;
                self.state = State::Data;

                return;
            };

            if content_start < close_pos {
                self.emit_token(TokenType::Text, content_start, close_pos);
            }

            self.emit_token(TokenType::CommentEnd, close_pos, close_pos + 3);
            self.pos = close_pos + 3;
            self.state = State::Data;

            return;
        }

        // EOF without closing. Emit remaining content and error
        if content_start < self.len {
            self.emit_token(TokenType::Text, content_start, self.len);
        }
        self.pos = self.len;
        self.log_error(State::Comment, ErrorReason::UnexpectedEof, self.len);
        self.state = State::Data;
    }

    /// `tryScanBogusComment`: `<?...>` (not a PHP tag) or `<-...->`. Restores the position and
    /// answers `false` at EOF, so it spends the distance it scanned.
    pub(super) fn try_scan_bogus_comment(&mut self) -> Result<bool, Exhausted> {
        if self.pos + 1 >= self.len {
            return Ok(false);
        }

        let start = self.pos;

        if self.source[self.pos] == b'<' && self.source[self.pos + 1] == b'?' {
            if self.php_tag_start_length(self.pos) > 0 {
                return Ok(false); // This is a PHP tag, not a bogus comment
            }

            self.pos += 2; // Skip the opening sequence

            // Bogus comments end at the first > character
            match self.source[self.pos.min(self.len)..self.len]
                .iter()
                .position(|&byte| byte == b'>')
            {
                Some(at) => {
                    self.pos += at + 1;
                    self.budget.spend((self.pos - start) as u64)?;
                    self.emit_token(TokenType::BogusComment, start, self.pos);

                    return Ok(true);
                }
                None => {
                    // EOF without closing. Restore position.
                    self.budget.spend((self.len - start) as u64)?;
                    self.pos = start;

                    return Ok(false);
                }
            }
        }

        // Check for <- ... ->
        if self.source[self.pos] == b'<' && self.source[self.pos + 1] == b'-' {
            // Make sure this is not <!--
            if self.at(self.pos + 2) == Some(b'-') {
                return Ok(false);
            }

            self.pos += 2; // Skip <-

            // Scan until ->
            while self.pos + 1 < self.len {
                if self.source[self.pos] == b'-' && self.source[self.pos + 1] == b'>' {
                    self.pos += 2;
                    self.budget.spend((self.pos - start) as u64)?;
                    self.emit_token(TokenType::BogusComment, start, self.pos);

                    return Ok(true);
                }
                self.pos += 1;
            }

            // EOF without closing. Restore position.
            self.budget.spend((self.pos - start) as u64)?;
            self.pos = start;

            return Ok(false);
        }

        Ok(false)
    }

    /// `tryScanConditionalComment`: `<!--[if ...]>`, and `<!-->` after it.
    pub(super) fn try_scan_conditional_comment(&mut self) -> bool {
        // Must start with <!--[
        if self.pos + 5 >= self.len {
            return false;
        }

        if &self.source[self.pos..self.pos + 5] != b"<!--[" {
            return false;
        }

        let start = self.pos;
        self.pos += 5; // Skip <!--[

        // Scan until we find "]>", which ends the conditional comment start
        while self.pos + 1 < self.len {
            if self.source[self.pos] == b']' && self.source[self.pos + 1] == b'>' {
                self.pos += 2;

                // Downlevel-hidden variant: ]><!-->
                if self.pos + 4 <= self.len
                    && &self.source[self.pos..self.pos + 4] == b"<!--"
                    && self.at(self.pos + 4) == Some(b'>')
                {
                    self.pos += 5; // Include <!--> in the token
                }

                self.emit_token(TokenType::ConditionalCommentStart, start, self.pos);

                return true;
            }
            self.pos += 1;
        }

        // EOF without closing "]>"
        self.pos = self.len;
        self.emit_token(TokenType::ConditionalCommentStart, start, self.pos);

        true
    }

    /// `tryScanConditionalCommentEnd`: `<![endif]-->`.
    pub(super) fn try_scan_conditional_comment_end(&mut self) -> bool {
        // Must start with <![
        if self.pos + 3 > self.len {
            return false;
        }

        if &self.source[self.pos..self.pos + 3] != b"<![" {
            return false;
        }

        let start = self.pos;
        self.pos += 3; // Skip <![

        self.scan_conditional_closing(start)
    }

    /// `tryScanDownlevelHiddenConditionalCommentEnd`: `<!--<![endif]-->`, one token.
    pub(super) fn try_scan_downlevel_hidden_conditional_comment_end(&mut self) -> bool {
        // Must start with <!--<![
        if self.pos + 7 > self.len {
            return false;
        }

        if &self.source[self.pos..self.pos + 7] != b"<!--<![" {
            return false;
        }

        let start = self.pos;
        self.pos += 7; // Skip <!--<![

        self.scan_conditional_closing(start)
    }

    /// `tryScanCdata`: `<![CDATA[...]]>`.
    pub(super) fn try_scan_cdata(&mut self) -> bool {
        // Must start with <![CDATA[
        if self.pos + 9 > self.len {
            return false;
        }

        if &self.source[self.pos..self.pos + 9] != b"<![CDATA[" {
            return false;
        }

        let start = self.pos;

        self.emit_token(TokenType::CdataStart, start, start + 9);
        self.pos += 9; // Skip <![CDATA[

        let content_start = self.pos;

        if self.pos < self.len {
            let Some(close_pos) = strpos(self.source, b"]]>", self.pos) else {
                // EOF. Unclosed CDATA
                if content_start < self.len {
                    self.emit_token(TokenType::Text, content_start, self.len);
                }
                self.log_error(State::Data, ErrorReason::UnexpectedEof, self.len);
                self.pos = self.len;

                return true;
            };

            if content_start < close_pos {
                self.emit_token(TokenType::Text, content_start, close_pos);
            }

            self.emit_token(TokenType::CdataEnd, close_pos, close_pos + 3);
            self.pos = close_pos + 3;

            return true;
        }

        // EOF without closing. Emit remaining content and error
        if content_start < self.len {
            self.emit_token(TokenType::Text, content_start, self.len);
        }
        self.pos = self.len;
        self.log_error(State::Data, ErrorReason::UnexpectedEof, self.len);

        true
    }

    /// `tryScanProcessingInstruction`: `<?target data?>` (not `<?xml`, not a PHP tag).
    pub(super) fn try_scan_processing_instruction(&mut self) -> bool {
        // Must start with <?
        if self.pos + 2 > self.len {
            return false;
        }

        if self.source[self.pos] != b'<' || self.source[self.pos + 1] != b'?' {
            return false;
        }

        // Check if followed by a valid PI target
        let Some(third_char) = self.at(self.pos + 2) else {
            return false;
        };

        // If space immediately after opening, it's a bogus comment, not PI
        if is_space(third_char) {
            return false;
        }

        if !is_alpha(third_char) {
            return false;
        }

        if self.php_tag_start_length(self.pos) > 0 {
            return false;
        }

        // Scan the target name
        let target_start = self.pos + 2;
        let mut target_end = target_start;
        while target_end < self.len {
            let c = self.source[target_end];
            if is_alnum(c) || matches!(c, b'-' | b'_' | b':') {
                target_end += 1;
            } else {
                break;
            }
        }

        if self.source[target_start..target_end].eq_ignore_ascii_case(b"xml") {
            return false;
        }

        let start = self.pos;

        let pi_start_end = target_end;

        self.emit_token(TokenType::PiStart, start, pi_start_end);
        self.pos = pi_start_end;

        let content_start = self.pos;

        // Scan for closing sequence
        while self.pos + 1 < self.len {
            if self.source[self.pos] == b'?' && self.source[self.pos + 1] == b'>' {
                if content_start < self.pos {
                    self.emit_token(TokenType::Text, content_start, self.pos);
                }

                self.emit_token(TokenType::PiEnd, self.pos, self.pos + 2);
                self.pos += 2;

                return true;
            }
            self.pos += 1;
        }

        // EOF without closing. Emit remaining content and error
        if content_start < self.len {
            self.emit_token(TokenType::Text, content_start, self.len);
        }
        self.pos = self.len;
        self.log_error(State::Data, ErrorReason::UnexpectedEof, self.len);

        true
    }

    /// `tryScanDecl`: `<?xml`, then attributes in `BeforeAttrName` until `?>`.
    pub(super) fn try_scan_decl(&mut self) -> bool {
        if self.pos + 5 > self.len {
            return false;
        }

        // Check for XML declaration (case-insensitive)
        if self.source[self.pos] != b'<' || self.source[self.pos + 1] != b'?' {
            return false;
        }

        if !self.source[self.pos + 2..self.pos + 5].eq_ignore_ascii_case(b"xml") {
            return false;
        }

        if let Some(next) = self.at(self.pos + 5)
            && (is_alnum(next) || matches!(next, b'_' | b'-' | b':'))
        {
            return false;
        }

        let start = self.pos;

        self.emit_token(TokenType::DeclStart, start, start + 5);
        self.pos += 5; // Skip <?xml

        self.in_xml_declaration = true;
        self.state = State::BeforeAttrName;

        true
    }

    /// `scanConditionalClosing`: up to and past `]-->`, or to EOF.
    fn scan_conditional_closing(&mut self, start: usize) -> bool {
        while self.pos + 3 < self.len {
            if &self.source[self.pos..self.pos + 4] == b"]-->" {
                self.pos += 4;
                self.emit_token(TokenType::ConditionalCommentEnd, start, self.pos);

                return true;
            }
            self.pos += 1;
        }

        // EOF without closing "]-->"
        self.pos = self.len;
        self.emit_token(TokenType::ConditionalCommentEnd, start, self.pos);

        true
    }
}
