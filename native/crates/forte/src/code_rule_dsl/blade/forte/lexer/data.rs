//! `Lexer/Concerns/DataScanner.php`.
use super::super::Exhausted;
use super::super::scan::{is_alnum, is_alpha};
use super::super::token::{ErrorReason, State, TokenType};
use super::Lexer;

impl Lexer<'_> {
    /// `scanData`.
    pub(super) fn scan_data(&mut self) -> Result<(), Exhausted> {
        let mut start = self.pos;
        let len = self.len;

        if !self.verbatim && !self.php_block && !self.php_tag {
            // `strcspn($this->source, '{<@'.$this->extensionTriggers, $this->pos)`; no extension
            // registers trigger characters.
            let skip = self.source[self.pos.min(len)..]
                .iter()
                .position(|byte| matches!(byte, b'{' | b'<' | b'@'))
                .unwrap_or(len.saturating_sub(self.pos));

            if skip > 0 {
                self.pos += skip;
                if self.pos >= len {
                    if start < self.pos {
                        self.emit_token(TokenType::Text, start, self.pos);
                    }

                    return Ok(());
                }

                // Stopping on an extension trigger: none are registered.
            }
        }

        // Scan until we hit a special character: {, <, or @
        while self.pos < len {
            let byte = self.source[self.pos];

            if byte == b'{' && !self.verbatim && !self.php_block && !self.php_tag {
                // Check if the previous character is @ (escaped: @{{ or @{!!)
                let prev_char = if self.pos > 0 {
                    self.at(self.pos - 1)
                } else {
                    None
                };
                if prev_char == Some(b'@') {
                    // Escaped echo, treat as text
                    self.pos += 1;

                    continue;
                }

                let next1 = self.at(self.pos + 1);

                // Check for Blade patterns: {{--, {{, {!!, {{{
                if next1 == Some(b'{') {
                    let next2 = self.at(self.pos + 2);
                    let next3 = self.at(self.pos + 3);
                    if start < self.pos {
                        self.emit_token(TokenType::Text, start, self.pos);
                    }

                    if next2 == Some(b'-') && next3 == Some(b'-') {
                        // Found {{--
                        self.scan_blade_comment_start();

                        return Ok(());
                    }

                    // Found {{ or {{{
                    self.scan_echo();

                    return Ok(());
                } else if next1 == Some(b'!') && self.at(self.pos + 2) == Some(b'!') {
                    // Found {!!
                    if start < self.pos {
                        self.emit_token(TokenType::Text, start, self.pos);
                    }
                    self.scan_raw_echo();

                    return Ok(());
                } else {
                    // Just a regular {, keep scanning
                    self.pos += 1;
                }
            } else if byte == b'@' {
                // In verbatim mode, only check for @endverbatim
                if self.verbatim {
                    if self.is_endverbatim_at(self.pos) {
                        if start < self.pos {
                            self.emit_token(TokenType::Text, start, self.pos);
                        }
                        self.scan_directive();

                        return Ok(());
                    }
                    // Not @endverbatim, keep it as text
                    self.pos += 1;

                    continue;
                }

                // In PHP block mode, only check for @endphp
                if self.php_block {
                    if self.is_endphp_at(self.pos) {
                        // Found @endphp - emit PHP block content before it
                        if start < self.pos {
                            self.emit_token(TokenType::PhpBlock, start, self.pos);
                        }
                        self.scan_directive();

                        return Ok(());
                    }
                    // Not @endphp, keep it as PHP block content
                    self.pos += 1;

                    continue;
                }

                // Only process @ if it can start a directive: not after an alphanumeric
                // (user@example.com), not after @ (escaped: @@directive)
                let prev_char = if self.pos > 0 {
                    self.at(self.pos - 1)
                } else {
                    None
                };
                let can_start =
                    self.pos == 0 || (!prev_char.is_some_and(is_alnum) && prev_char != Some(b'@'));

                if can_start {
                    if self.php_tag {
                        // In PHP tag mode, @ is just regular PHP content
                        self.pos += 1;
                    } else {
                        // Check if this is an escape sequence: @@, @{{, @{!!, or @{{{
                        let next_pos = self.pos + 1;
                        let mut is_escaped = false;
                        if next_pos < self.len {
                            let next_byte = self.source[next_pos];
                            if next_byte == b'@' {
                                // @@directive
                                is_escaped = true;
                            } else if next_byte == b'{' && next_pos + 1 < self.len {
                                // Check for @{{, @{!!, or @{{{
                                let after_brace = self.source[next_pos + 1];
                                if after_brace == b'{' || after_brace == b'!' {
                                    is_escaped = true;
                                }
                            }
                        }

                        if start < self.pos {
                            self.emit_token(TokenType::Text, start, self.pos);
                        }

                        if is_escaped {
                            self.emit_token(TokenType::AtSign, self.pos, self.pos + 1);
                            self.pos += 1;

                            return Ok(());
                        }

                        self.scan_directive();

                        return Ok(());
                    }
                } else {
                    // @ can't start directive (e.g., in email), keep scanning as text
                    self.pos += 1;
                }
            } else if byte == b'<' && !self.verbatim && !self.php_block && !self.php_tag {
                let pos = self.pos;
                let source = self.source;

                // Check for DOCTYPE
                if pos + 9 <= len
                    && source[pos + 1] == b'!'
                    && source[pos + 2..pos + 9].eq_ignore_ascii_case(b"doctype")
                {
                    if start < pos {
                        self.emit_token(TokenType::Text, start, pos);
                    }
                    self.scan_doctype();

                    return Ok(());
                }

                // Check for CDATA section, conditional comment end, or other <![...] patterns
                if pos + 3 <= len && source[pos + 1] == b'!' && source[pos + 2] == b'[' {
                    if start < pos {
                        self.emit_token(TokenType::Text, start, pos);
                    }

                    // Try CDATA first: <![CDATA[
                    if self.try_scan_cdata() {
                        return Ok(());
                    }

                    // Otherwise, try conditional comment end: <![endif]-->
                    self.try_scan_conditional_comment_end();

                    return Ok(());
                }

                // Check for downlevel-hidden conditional comment end: <!--<![endif]-->
                if pos + 7 <= len && &source[pos + 1..pos + 7] == b"!--<![" {
                    if start < pos {
                        self.emit_token(TokenType::Text, start, pos);
                    }

                    self.try_scan_downlevel_hidden_conditional_comment_end();

                    return Ok(());
                }

                // Check for the standard HTML comment: <!--
                if pos + 4 <= len && &source[pos + 1..pos + 4] == b"!--" {
                    if start < pos {
                        self.emit_token(TokenType::Text, start, pos);
                    }

                    // Check for conditional comment: <!--[if ...]>
                    if self.try_scan_conditional_comment() {
                        return Ok(());
                    }

                    // Standard HTML comment <!-- ... -->
                    self.state = State::Comment;

                    return Ok(());
                }

                // Check for bogus comment with single dash: <!- (not <!--)
                if pos + 3 <= len
                    && source[pos + 1] == b'!'
                    && source[pos + 2] == b'-'
                    && (pos + 3 >= len || source[pos + 3] != b'-')
                {
                    if start < pos {
                        self.emit_token(TokenType::Text, start, pos);
                    }

                    let bogus_start = pos; // Scan until -> or >
                    self.pos += 3; // Skip <!-
                    while self.pos < len {
                        if source[self.pos] == b'>' {
                            self.pos += 1;
                            self.emit_token(TokenType::BogusComment, bogus_start, self.pos);

                            return Ok(());
                        }
                        if self.pos + 1 < len
                            && source[self.pos] == b'-'
                            && source[self.pos + 1] == b'>'
                        {
                            self.pos += 2;
                            self.emit_token(TokenType::BogusComment, bogus_start, self.pos);

                            return Ok(());
                        }
                        self.pos += 1;
                    }
                    self.emit_token(TokenType::BogusComment, bogus_start, self.pos);

                    return Ok(());
                }

                // Check for XML declaration first: <?xml
                let before_decl = self.pos;
                if start < before_decl {
                    self.emit_token(TokenType::Text, start, before_decl);
                    start = before_decl;
                }
                if self.try_scan_decl() {
                    return Ok(());
                }

                // Check for Processing Instruction: <?target data
                let before_pi = self.pos;
                if start < before_pi {
                    self.emit_token(TokenType::Text, start, before_pi);
                    start = before_pi;
                }
                if self.try_scan_processing_instruction() {
                    return Ok(());
                }

                // Check for bogus comments: <? ... or <- ...
                let before_bogus = self.pos;
                if self.try_scan_bogus_comment()? {
                    if start < before_bogus {
                        self.emit_token(TokenType::Text, start, before_bogus);
                    }

                    return Ok(());
                }

                // Check for PHP tag start: <?php, <?=, or <?
                let tag_len = self.php_tag_start_length(self.pos);
                if tag_len > 0 {
                    if start < self.pos {
                        self.emit_token(TokenType::Text, start, self.pos);
                    }
                    self.emit_token(TokenType::PhpTagStart, self.pos, self.pos + tag_len);
                    self.pos += tag_len;
                    self.php_tag = true;

                    return Ok(());
                }

                let next_char = self.at(self.pos + 1);
                let is_valid_tag_start = next_char.is_some_and(|next| {
                    is_alpha(next) || matches!(next, b'/' | b'_' | b'>' | b'{' | b'@')
                });

                if is_valid_tag_start {
                    if start < self.pos {
                        self.emit_token(TokenType::Text, start, self.pos);
                    }
                    self.scan_tag_open();

                    return Ok(());
                }
                self.pos += 1;
            } else if byte == b'?' && self.php_tag {
                if self.is_php_tag_end_at(self.pos) {
                    if start < self.pos {
                        self.emit_token(TokenType::PhpContent, start, self.pos);
                    }
                    self.emit_token(TokenType::PhpTagEnd, self.pos, self.pos + 2);
                    self.pos += 2;
                    self.php_tag = false;

                    return Ok(());
                }
                // Just a regular ?, keep scanning
                self.pos += 1;
            } else {
                self.pos += 1;
            }
        }

        if start < self.pos {
            if self.php_block {
                self.log_error(State::Data, ErrorReason::UnexpectedEof, self.pos);
                self.emit_token(TokenType::PhpBlock, start, self.pos);
            } else if self.php_tag {
                self.emit_token(TokenType::PhpContent, start, self.pos);
            } else if self.verbatim {
                self.log_error(State::Data, ErrorReason::UnexpectedEof, self.pos);
                self.emit_token(TokenType::Text, start, self.pos);
            } else {
                self.emit_token(TokenType::Text, start, self.pos);
            }
        }

        Ok(())
    }
}
