//! `Lexer/Concerns/AttributeScanner.php`. `handlePostAttributeName` runs only after an attribute
//! extension matched, and none are registered, so it is left out.
use super::super::Exhausted;
use super::super::scan::{is_alnum, is_space};
use super::super::token::{State, TokenType};
use super::Lexer;

impl Lexer<'_> {
    /// `scanAttrName`.
    pub(super) fn scan_attr_name(&mut self) {
        let mut start = self.pos;

        // `tryAttributeExtension()`: no attribute extensions are registered.

        // Check for special Blade attribute patterns at the start
        let mut token_type = TokenType::AttributeName;

        if self.pos < self.len && self.source[self.pos] == b':' {
            token_type = match self.at(self.pos + 1) {
                // ::name
                Some(b':') => TokenType::EscapedAttribute,
                // :$name
                Some(b'$') => TokenType::ShorthandAttribute,
                // :name
                _ => TokenType::BoundAttribute,
            };
        }

        loop {
            if self.pos >= self.len {
                break;
            }

            let byte = self.source[self.pos];

            // Check for Blade echo in the attribute name (e.g., class-{{ $thing }})
            if byte == b'{' && !self.verbatim && !self.php_block && !self.php_tag {
                // Emit attribute name so far if we have any
                if self.pos > start {
                    self.emit_token(token_type, start, self.pos);
                    start = self.pos;
                }

                let saved_pos = self.pos;
                if self.peek_ahead(1) == Some(b'{') {
                    let saved_state = self.state;

                    // Check for {{-- (blade comment)
                    if self.peek_ahead(2) == Some(b'-') && self.peek_ahead(3) == Some(b'-') {
                        self.return_state = State::Data;
                        self.scan_blade_comment_start();
                        self.scan_blade_comment_content();
                        self.state = saved_state;

                        start = self.pos;

                        continue;
                    }

                    // Scan {{ or {{{ echo
                    self.return_state = State::Data;
                    self.scan_echo();
                    self.state = saved_state;

                    start = self.pos;

                    continue;
                } else if self.peek_ahead(1) == Some(b'!') && self.peek_ahead(2) == Some(b'!') {
                    // Scan {!! raw echo
                    let saved_state = self.state;
                    self.return_state = State::Data;
                    self.scan_raw_echo();
                    self.state = saved_state;

                    start = self.pos;

                    continue;
                }

                // Not a Blade echo. Restore position and break (end of attribute name)
                self.pos = saved_pos;
                break;
            }

            // Check for PHP tag in the attribute name (e.g., data-<?php echo 'thing'; ?>more)
            if byte == b'<' && self.pos + 1 < self.len && self.source[self.pos + 1] == b'?' {
                if self.pos > start {
                    self.emit_token(token_type, start, self.pos);
                    start = self.pos;
                }

                if self.try_scan_php_tag() {
                    start = self.pos;

                    continue;
                }

                break;
            }

            // Check for Blade directive in the attribute name (e.g., class-@if($x)thing)
            if byte == b'@' && !self.verbatim && !self.php_block && !self.php_tag {
                let mut temp_pos = self.pos + 1; // Skip @
                let name_start = temp_pos;

                while temp_pos < self.len {
                    let dir_byte = self.source[temp_pos];
                    if is_alnum(dir_byte) || dir_byte == b'_' {
                        temp_pos += 1;
                    } else {
                        break;
                    }
                }

                let is_known_directive = temp_pos > name_start
                    && self
                        .directives
                        .is_directive(&self.source[name_start..temp_pos]);

                if is_known_directive && !self.has_modifier_call_at(temp_pos) {
                    if self.pos > start {
                        self.emit_token(token_type, start, self.pos);
                    }

                    let saved_state = self.state;
                    self.return_state = State::Data;
                    self.scan_directive();
                    self.state = saved_state;

                    start = self.pos;

                    continue;
                }

                self.pos += 1;

                continue;
            }

            // Keep modifier call payloads opaque to tag-closing detection.
            if byte == b'(' && start < self.pos && self.source[start] == b'@' {
                self.skip_balanced_parens();

                continue;
            }

            // Regular attribute name characters; @ and . for Vue/Alpine (@click.prevent.stop)
            if is_alnum(byte) || matches!(byte, b'-' | b':' | b'_' | b'$' | b'@' | b'.') {
                self.pos += 1;
            } else {
                break;
            }
        }

        if start < self.pos {
            self.emit_token(token_type, start, self.pos);
        }

        if self.pos >= self.len {
            // EOF. Unclosed tag
            self.state = State::Data;

            return;
        }

        let ch = self.source[self.pos];

        if ch == b'=' {
            // Attribute has a value
            self.emit_token(TokenType::Equals, self.pos, self.pos + 1);
            self.pos += 1;
            self.state = State::BeforeAttrValue;
        } else if self.in_xml_declaration && ch == b'?' && self.at(self.pos + 1) == Some(b'>') {
            // Boolean attribute, end of XML declaration
            self.emit_token(TokenType::DeclEnd, self.pos, self.pos + 2);
            self.pos += 2;
            self.in_xml_declaration = false;
            self.state = State::Data;
        } else if ch == b'>' {
            // Boolean attribute, end of tag
            self.emit_token(TokenType::GreaterThan, self.pos, self.pos + 1);
            self.pos += 1;
            self.state = self.state_after_tag_close();
        } else if ch == b'/' {
            // Boolean attribute, self-closing tag
            self.emit_token(TokenType::Slash, self.pos, self.pos + 1);
            self.pos += 1;

            if self.pos < self.len && self.source[self.pos] == b'>' {
                self.emit_token(TokenType::GreaterThan, self.pos, self.pos + 1);
                self.pos += 1;
                self.state = State::Data;
            } else {
                // Malformed, but continue
                self.state = State::BeforeAttrName;
            }
        } else if is_space(ch) {
            // Whitespace after attribute name
            self.state = State::AfterAttrName;
        } else if ch == b'<' {
            // A new tag starting without closing the current one (e.g. TSX generics:
            // <Map<Record<); BeforeAttrName emits SyntheticClose if needed
            self.state = State::BeforeAttrName;
        } else {
            // Unexpected character. Emit as text so we can recover it later
            self.emit_token(TokenType::Text, self.pos, self.pos + 1);
            self.pos += 1;
            self.state = State::BeforeAttrName;
        }
    }

    /// `hasModifierCallAt`: `.segment(.segment)*` then optional whitespace then `(`.
    pub(super) fn has_modifier_call_at(&self, mut pos: usize) -> bool {
        if pos >= self.len || self.source[pos] != b'.' {
            return false;
        }

        while pos < self.len && self.source[pos] == b'.' {
            pos += 1;
            let segment_start = pos;

            while pos < self.len {
                let byte = self.source[pos];

                if !is_alnum(byte) && byte != b'_' {
                    break;
                }

                pos += 1;
            }

            if pos == segment_start {
                return false;
            }
        }

        while pos < self.len && is_space(self.source[pos]) {
            pos += 1;
        }

        pos < self.len && self.source[pos] == b'('
    }

    /// `scanAfterAttrName`.
    pub(super) fn scan_after_attr_name(&mut self) {
        self.skip_and_emit_whitespace();

        if self.pos >= self.len {
            // EOF. Unclosed tag
            self.state = State::Data;

            return;
        }

        let ch = self.source[self.pos];

        if ch == b'=' {
            self.emit_token(TokenType::Equals, self.pos, self.pos + 1);
            self.pos += 1;
            self.state = State::BeforeAttrValue;
        } else if self.in_xml_declaration && ch == b'?' && self.at(self.pos + 1) == Some(b'>') {
            // Boolean attribute, end of XML declaration
            self.emit_token(TokenType::DeclEnd, self.pos, self.pos + 2);
            self.pos += 2;
            self.in_xml_declaration = false;
            self.state = State::Data;
        } else if ch == b'>' {
            // Boolean attribute, end of tag
            self.emit_token(TokenType::GreaterThan, self.pos, self.pos + 1);
            self.pos += 1;
            self.state = self.state_after_tag_close();
        } else if ch == b'/' {
            // Boolean attribute, self-closing tag
            self.emit_token(TokenType::Slash, self.pos, self.pos + 1);
            self.pos += 1;

            if self.pos < self.len && self.source[self.pos] == b'>' {
                self.emit_token(TokenType::GreaterThan, self.pos, self.pos + 1);
                self.pos += 1;
                self.state = State::Data;
            } else {
                // Malformed
                self.state = State::BeforeAttrName;
            }
        } else {
            // Some other character; we'll let BeforeAttrName handle it
            self.state = State::BeforeAttrName;
        }
    }

    /// `scanBeforeAttrValue`.
    pub(super) fn scan_before_attr_value(&mut self) -> Result<(), Exhausted> {
        self.skip_and_emit_whitespace();

        if self.pos >= self.len {
            // EOF. Unclosed tag
            self.state = State::Data;

            return Ok(());
        }

        let ch = self.source[self.pos];

        // Prioritize Blade echoes over JSX-style attributes
        if ch == b'{' && !self.verbatim && !self.php_block && !self.php_tag {
            if self.peek_ahead(1) == Some(b'{') {
                // {{ or {{{
                self.return_state = State::BeforeAttrName;
                self.scan_echo();
                self.state = State::BeforeAttrName;

                return Ok(());
            } else if self.peek_ahead(1) == Some(b'!') && self.peek_ahead(2) == Some(b'!') {
                // {!!
                self.return_state = State::BeforeAttrName;
                self.scan_raw_echo();
                self.state = State::BeforeAttrName;

                return Ok(());
            }

            // Not a Blade echo, try JSX-style attribute value
            if self.scan_jsx_attribute_value()? {
                self.state = State::BeforeAttrName;

                return Ok(());
            }

            // Not JSX, treat as unquoted value
            self.state = State::AttrValueUnquoted;

            return Ok(());
        }

        // Check for ({expression}) pattern
        if ch == b'(' && self.at(self.pos + 1) == Some(b'{') {
            if self.scan_jsx_attribute_value()? {
                self.state = State::BeforeAttrName;

                return Ok(());
            }

            self.state = State::AttrValueUnquoted;

            return Ok(());
        }

        if ch == b'"' || ch == b'\'' {
            self.state = State::AttrValueQuoted;
        } else if self.in_xml_declaration && ch == b'?' && self.at(self.pos + 1) == Some(b'>') {
            // Empty attribute value, end of XML declaration
            self.emit_token(TokenType::DeclEnd, self.pos, self.pos + 2);
            self.pos += 2;
            self.in_xml_declaration = false;
            self.state = State::Data;
        } else if ch == b'>' {
            // Empty attribute value, end of tag
            self.emit_token(TokenType::GreaterThan, self.pos, self.pos + 1);
            self.pos += 1;
            self.state = self.state_after_tag_close();
        } else {
            self.state = State::AttrValueUnquoted;
        }

        Ok(())
    }

    /// `scanAttrValueQuoted`.
    pub(super) fn scan_attr_value_quoted(&mut self) {
        let start = self.pos;
        let Some(quote) = self.at(self.pos) else {
            return;
        };

        // Emit the opening quote and skip.
        self.emit_token(TokenType::Quote, start, start + 1);
        self.pos += 1;

        let mut value_start = self.pos;

        loop {
            if self.pos >= self.len {
                // EOF
                if self.pos > value_start {
                    self.emit_token(TokenType::AttributeValue, value_start, self.pos);
                }
                self.state = State::BeforeAttrName;

                return;
            }

            // The next quote, {, or @: the least of PHP's three `strpos` calls, found in one pass.
            let next_pos = self.source[self.pos..self.len]
                .iter()
                .position(|&byte| byte == quote || byte == b'{' || byte == b'@')
                .map_or(self.len, |at| self.pos + at);

            if next_pos == self.len {
                // No quote, {, or @ found
                self.pos = self.len;
                if self.pos > value_start {
                    self.emit_token(TokenType::AttributeValue, value_start, self.pos);
                }
                self.state = State::BeforeAttrName;

                return;
            }

            self.pos = next_pos;
            let byte = self.source[self.pos];

            if byte == quote {
                // Check if this quote is escaped
                let mut backslash_count = 0usize;
                let mut check = self.pos;
                while check > value_start && self.source[check - 1] == b'\\' {
                    backslash_count += 1;
                    check -= 1;
                }

                if backslash_count % 2 == 1 {
                    self.pos += 1;

                    continue;
                }

                // Found unescaped closing quote
                if self.pos > value_start {
                    self.emit_token(TokenType::AttributeValue, value_start, self.pos);
                }

                self.emit_token(TokenType::Quote, self.pos, self.pos + 1);
                self.pos += 1;
                self.state = State::BeforeAttrName;

                return;
            } else if byte == b'{' && !self.verbatim && !self.php_block && !self.php_tag {
                // Potential echo delimiter
                let saved_pos = self.pos;

                if self.peek_ahead(1) == Some(b'{') {
                    // Check for {{-- (blade comment)
                    if self.peek_ahead(2) == Some(b'-') && self.peek_ahead(3) == Some(b'-') {
                        if saved_pos > value_start {
                            self.emit_token(TokenType::AttributeValue, value_start, saved_pos);
                        }
                        self.return_state = State::AttrValueQuoted;
                        self.scan_blade_comment_start();
                        self.scan_blade_comment_content();
                        value_start = self.pos; // Resume after comment

                        continue;
                    }
                    // Found {{ or {{{
                    if saved_pos > value_start {
                        self.emit_token(TokenType::AttributeValue, value_start, saved_pos);
                    }
                    self.return_state = State::AttrValueQuoted;
                    self.scan_echo();
                    value_start = self.pos; // Resume after echo

                    continue;
                } else if self.peek_ahead(1) == Some(b'!') && self.peek_ahead(2) == Some(b'!') {
                    // Found {!!
                    if saved_pos > value_start {
                        self.emit_token(TokenType::AttributeValue, value_start, saved_pos);
                    }
                    self.return_state = State::AttrValueQuoted;
                    self.scan_raw_echo();
                    value_start = self.pos; // Resume after echo

                    continue;
                }

                // Not an echo, just a regular {
                self.pos = saved_pos + 1;
            } else if byte == b'@' && !self.verbatim && !self.php_block && !self.php_tag {
                // Potential directive: @ can start one unless it follows an alphanumeric
                let can_start =
                    self.pos == value_start || !self.at(self.pos - 1).is_some_and(is_alnum);

                if can_start {
                    let saved_pos = self.pos;

                    if saved_pos > value_start {
                        self.emit_token(TokenType::AttributeValue, value_start, saved_pos);
                    }

                    self.return_state = State::AttrValueQuoted;
                    self.scan_directive();
                    value_start = self.pos; // Resume after directive

                    continue;
                }

                // Not a directive, just a regular @
                self.pos += 1;
            } else {
                // Fail-safe. We _shouldn't_ get here, but the real world is messy
                self.pos += 1;
            }
        }
    }

    /// `scanAttrValueUnquoted`.
    pub(super) fn scan_attr_value_unquoted(&mut self) {
        let mut value_start = self.pos;

        // Scan until whitespace, >, XML declarations, or EOF
        loop {
            if self.pos >= self.len {
                break;
            }

            let byte = self.source[self.pos];

            // Check for XML declaration ending
            if self.in_xml_declaration && byte == b'?' && self.at(self.pos + 1) == Some(b'>') {
                break;
            }

            // Check for the end of an unquoted value
            if matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b'>') {
                break;
            }

            // Check for PHP tag in unquoted value
            if byte == b'<' && self.at(self.pos + 1) == Some(b'?') {
                let saved_pos = self.pos;

                if saved_pos > value_start {
                    self.emit_token(TokenType::AttributeValue, value_start, saved_pos);
                }

                if self.try_scan_php_tag() {
                    value_start = self.pos;

                    continue;
                }

                // Not a PHP tag. Keep valueStart aligned with emitted content.
                value_start = saved_pos;
                break;
            }

            // Check for Blade echo in unquoted value
            if byte == b'{' && !self.verbatim && !self.php_block && !self.php_tag {
                let saved_pos = self.pos;

                if self.peek_ahead(1) == Some(b'{') {
                    // Check for {{-- (blade comment)
                    if self.peek_ahead(2) == Some(b'-') && self.peek_ahead(3) == Some(b'-') {
                        if saved_pos > value_start {
                            self.emit_token(TokenType::AttributeValue, value_start, saved_pos);
                        }
                        self.return_state = State::AttrValueUnquoted;
                        self.scan_blade_comment_start();
                        self.scan_blade_comment_content();
                        value_start = self.pos;

                        continue;
                    }
                    // Found {{ or {{{
                    if saved_pos > value_start {
                        self.emit_token(TokenType::AttributeValue, value_start, saved_pos);
                    }
                    self.return_state = State::AttrValueUnquoted;
                    self.scan_echo();
                    value_start = self.pos;

                    continue;
                } else if self.peek_ahead(1) == Some(b'!') && self.peek_ahead(2) == Some(b'!') {
                    // Found {!!
                    if saved_pos > value_start {
                        self.emit_token(TokenType::AttributeValue, value_start, saved_pos);
                    }
                    self.return_state = State::AttrValueUnquoted;
                    self.scan_raw_echo();
                    value_start = self.pos;

                    continue;
                }

                // Not an echo, just a regular {
                self.pos = saved_pos + 1;

                continue;
            }

            // Check for Blade directive in unquoted value
            if byte == b'@' && !self.verbatim && !self.php_block && !self.php_tag {
                let saved_pos = self.pos;

                let can_start =
                    self.pos == value_start || !self.at(self.pos - 1).is_some_and(is_alnum);

                if can_start {
                    if saved_pos > value_start {
                        self.emit_token(TokenType::AttributeValue, value_start, saved_pos);
                    }

                    self.return_state = State::AttrValueUnquoted;
                    self.scan_directive();
                    value_start = self.pos;

                    continue;
                }

                // Not a directive, just a regular @
                self.pos += 1;

                continue;
            }

            self.pos += 1;
        }

        if value_start < self.pos {
            self.emit_token(TokenType::AttributeValue, value_start, self.pos);
        }

        self.state = State::BeforeAttrName;
    }
}
