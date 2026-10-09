//! `Lexer/Concerns/RawtextScanner.php`.
use super::super::scan::{is_alnum, is_space};
use super::super::token::{State, TokenType};
use super::Lexer;

impl Lexer<'_> {
    /// `scanRawtext`: the content of a raw text element, up to its closing tag.
    pub(super) fn scan_rawtext(&mut self) {
        let start = self.pos;
        let len = self.len;
        let tag_name = self.rawtext_tag_name;
        let tag_name_len = tag_name.len();

        while self.pos < len {
            let byte = self.source[self.pos];

            // Check for Blade constructs
            if byte == b'{' && !self.verbatim && !self.php_block && !self.php_tag {
                let next1 = self.at(self.pos + 1);

                // Check for Blade patterns: {{--, {{, {!!, {{{
                if next1 == Some(b'{') {
                    let next2 = self.at(self.pos + 2);
                    let next3 = self.at(self.pos + 3);
                    if start < self.pos {
                        self.emit_token(TokenType::Text, start, self.pos);
                    }

                    self.return_state = State::RawText;
                    if next2 == Some(b'-') && next3 == Some(b'-') {
                        // Found {{-- blade comment
                        self.scan_blade_comment_start();

                        return;
                    }
                    // Found {{ or {{{
                    self.scan_echo();

                    return;
                } else if next1 == Some(b'!') && self.at(self.pos + 2) == Some(b'!') {
                    // Found {!!
                    if start < self.pos {
                        self.emit_token(TokenType::Text, start, self.pos);
                    }
                    self.return_state = State::RawText;
                    self.scan_raw_echo();

                    return;
                }
                // Just a regular {
                self.pos += 1;

                continue;
            }

            // Directive stuff.
            if byte == b'@' && !self.verbatim && !self.php_block && !self.php_tag {
                let prev_char = if self.pos > 0 {
                    self.at(self.pos - 1)
                } else {
                    None
                };
                let can_start =
                    self.pos == 0 || (!prev_char.is_some_and(is_alnum) && prev_char != Some(b'@'));

                if can_start {
                    // Check for escape sequences: @@, @{{, @{!!
                    let next_pos = self.pos + 1;
                    let mut is_escaped = false;
                    if next_pos < self.len {
                        let next_byte = self.source[next_pos];
                        if next_byte == b'@' {
                            is_escaped = true;
                        } else if next_byte == b'{' && next_pos + 1 < self.len {
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

                        return;
                    }

                    self.return_state = State::RawText;
                    self.scan_directive();

                    return;
                }
                self.pos += 1;

                continue;
            }

            // Check for this element's closing tag.
            if byte == b'<' {
                if self.pos + 2 + tag_name_len <= len && self.source[self.pos + 1] == b'/' {
                    let name_at = self.pos + 2;
                    let potential_tag_name = &self.source[name_at..name_at + tag_name_len];
                    if potential_tag_name.eq_ignore_ascii_case(tag_name) {
                        // Check that tag name ends at word boundary (whitespace or >)
                        let after_tag_pos = name_at + tag_name_len;
                        if after_tag_pos < len {
                            let after_tag_char = self.source[after_tag_pos];
                            if after_tag_char == b'>' || is_space(after_tag_char) {
                                if start < self.pos {
                                    self.emit_token(TokenType::Text, start, self.pos);
                                }

                                // Exit rawtext mode
                                self.rawtext_tag_name = b"";

                                // Emit the closing tag tokens: <, /, tagname
                                self.emit_token(TokenType::LessThan, self.pos, self.pos + 1);
                                self.pos += 1;
                                self.emit_token(TokenType::Slash, self.pos, self.pos + 1);
                                self.pos += 1;
                                self.emit_token(
                                    TokenType::TagName,
                                    self.pos,
                                    self.pos + tag_name_len,
                                );
                                self.current_tag_name.clear();
                                self.current_tag_name.extend_from_slice(potential_tag_name);
                                self.is_closing_tag = true;
                                self.pos += tag_name_len;

                                // Look ahead past whitespace to check for >
                                let mut peek_pos = self.pos;
                                while peek_pos < self.len && is_space(self.source[peek_pos]) {
                                    peek_pos += 1;
                                }

                                // Only consume whitespace if followed by >
                                if peek_pos < self.len && self.source[peek_pos] == b'>' {
                                    if self.pos < peek_pos {
                                        self.emit_token(TokenType::Whitespace, self.pos, peek_pos);
                                        self.pos = peek_pos;
                                    }
                                    self.emit_token(TokenType::GreaterThan, self.pos, self.pos + 1);
                                    self.pos += 1;
                                } else {
                                    // A malformed closing tag (no >): SyntheticClose, whitespace
                                    // left alone.
                                    self.emit_token(TokenType::SyntheticClose, self.pos, self.pos);
                                }
                                self.state = State::Data;

                                return;
                            }
                        } else if after_tag_pos == len {
                            // Tag names at EOF will just become text
                            if start < self.pos {
                                self.emit_token(TokenType::Text, start, self.pos);
                            }
                            self.rawtext_tag_name = b"";
                            self.emit_token(TokenType::LessThan, self.pos, self.pos + 1);
                            self.pos += 1;
                            self.emit_token(TokenType::Slash, self.pos, self.pos + 1);
                            self.pos += 1;
                            self.emit_token(TokenType::TagName, self.pos, self.pos + tag_name_len);
                            self.pos += tag_name_len;
                            self.state = State::Data;

                            return;
                        }
                    }
                }

                // Treat < as text since its not a closing tag.
                self.pos += 1;

                continue;
            }

            self.pos += 1;
        }

        // EOF. Emit remaining content as text
        if start < self.pos {
            self.emit_token(TokenType::Text, start, self.pos);
        }

        // Exit rawtext mode (unclosed element)
        self.rawtext_tag_name = b"";
    }
}
