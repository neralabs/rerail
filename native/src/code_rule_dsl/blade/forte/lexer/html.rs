//! `Lexer/Concerns/HtmlScanner.php`.
use super::super::Exhausted;
use super::super::scan::{is_alnum, is_space};
use super::super::token::{State, TokenType};
use super::Lexer;

/// `SPECIAL_TEXT_ELEMENTS`: the elements whose content is raw text (or RCDATA).
const SPECIAL_TEXT_ELEMENTS: [&[u8]; 8] = [
    b"iframe",
    b"noembed",
    b"noframes",
    b"script",
    b"style",
    b"textarea",
    b"title",
    b"xmp",
];

/// A byte `scanTagName` takes into a tag name.
fn is_tag_name_byte(byte: u8) -> bool {
    is_alnum(byte) || matches!(byte, b'-' | b':' | b'_' | b'@' | b'.' | b'[' | b']' | b'$')
}

impl Lexer<'_> {
    /// `stateAfterTagClose`.
    pub(super) fn state_after_tag_close(&mut self) -> State {
        if self.is_closing_tag {
            return State::Data;
        }

        // `isset(self::SPECIAL_TEXT_ELEMENTS[strtolower($this->currentTagName)])`.
        for name in SPECIAL_TEXT_ELEMENTS {
            if self.current_tag_name.eq_ignore_ascii_case(name) {
                self.rawtext_tag_name = name;

                return State::RawText;
            }
        }

        State::Data
    }

    /// `scanTagOpen`.
    pub(super) fn scan_tag_open(&mut self) {
        let start = self.pos;

        // Reset tag tracking for new tag
        self.current_tag_name.clear();

        self.emit_token(TokenType::LessThan, start, start + 1);
        self.pos += 1; // Skip <

        // Check for closing tag (</...)
        if self.pos < self.len && self.source[self.pos] == b'/' {
            self.emit_token(TokenType::Slash, self.pos, self.pos + 1);
            self.pos += 1;
            self.is_closing_tag = true;
        } else {
            self.is_closing_tag = false;
        }

        self.state = State::TagName;
    }

    /// `scanTagName`.
    pub(super) fn scan_tag_name(&mut self) -> Result<(), Exhausted> {
        let start = self.pos;

        // Continuing a composite tag name (returned from Blade): whitespace ends the name.
        if self.continued_tag_name && self.pos < self.len && is_space(self.source[self.pos]) {
            self.continued_tag_name = false;
            self.state = State::BeforeAttrName;

            return Ok(());
        }

        // Skip whitespace before tag name (for malformed HTML like "< div>"), only at the
        // initial entry
        if !self.continued_tag_name {
            self.skip_whitespace();
        }

        self.continued_tag_name = false;

        // If we hit > or / immediately, it's an empty/malformed tag like "<>" or "</>"
        if self.pos >= self.len || self.source[self.pos] == b'>' || self.source[self.pos] == b'/' {
            if start < self.pos {
                self.emit_token(TokenType::Whitespace, start, self.pos);
            }

            if self.pos < self.len {
                if self.source[self.pos] == b'>' {
                    self.emit_token(TokenType::GreaterThan, self.pos, self.pos + 1);
                    self.pos += 1;
                    self.state = self.state_after_tag_close();
                } else if self.source[self.pos] == b'/' {
                    // Self-closing tag (never enters rawtext mode)
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
                }
            } else {
                // EOF. Unclosed tag, emit SyntheticClose for error recovery
                self.emit_token(TokenType::SyntheticClose, self.pos, self.pos);
                self.state = State::Data;
            }

            return Ok(());
        }

        // Malformed closing tag with leading whitespace before a Blade echo-like sequence (e.g.
        // "</\r{{"): close the tag and rewind so the skipped whitespace is data.
        if self.is_closing_tag
            && start < self.pos
            && self.pos < self.len
            && self.source[self.pos] == b'{'
        {
            self.emit_token(TokenType::SyntheticClose, start, start);
            self.pos = start;
            self.state = State::Data;

            return Ok(());
        }

        let name_start = self.pos;

        while self.pos < self.len {
            let ch = self.source[self.pos];

            // Check for Blade echo in the tag name (e.g., <{{ $element }}>)
            if ch == b'{' && !self.verbatim && !self.php_block && !self.php_tag {
                let saved_pos = self.pos;
                if self.peek_ahead(1) == Some(b'{') {
                    if start < name_start {
                        self.emit_token(TokenType::Whitespace, start, name_start);
                    }

                    if self.pos > name_start {
                        self.emit_token(TokenType::TagName, name_start, self.pos);
                    }

                    // Check for {{--
                    if self.peek_ahead(2) == Some(b'-') && self.peek_ahead(3) == Some(b'-') {
                        self.return_state = State::TagName;
                        self.continued_tag_name = true;
                        self.scan_blade_comment_start();
                        self.scan_blade_comment_content();

                        return Ok(());
                    }

                    // Scan {{ or {{{ echo
                    self.return_state = State::TagName;
                    self.continued_tag_name = true;
                    self.scan_echo();

                    return Ok(());
                } else if self.peek_ahead(1) == Some(b'!') && self.peek_ahead(2) == Some(b'!') {
                    if start < name_start {
                        self.emit_token(TokenType::Whitespace, start, name_start);
                    }

                    if self.pos > name_start {
                        self.emit_token(TokenType::TagName, name_start, self.pos);
                    }

                    // Scan {!! raw echo
                    self.return_state = State::TagName;
                    self.continued_tag_name = true;
                    self.scan_raw_echo();

                    return Ok(());
                }

                // Not a Blade echo. Restore and break (end of tag name)
                self.pos = saved_pos;
                break;
            }

            // Tag name characters, including [] and $ for slot syntax (<x-slot[name]>)
            if is_tag_name_byte(ch) {
                self.pos += 1;
            } else {
                break;
            }
        }

        // Preserve whitespace between < and a recovered tag name ("<\nname", "</ @e").
        if start < name_start && name_start < self.pos {
            self.emit_token(TokenType::Whitespace, start, name_start);
        }

        // Preserve skipped leading whitespace when no tag-name bytes were consumed and the next
        // byte cannot start a valid name/construct.
        if start < name_start && name_start == self.pos && self.pos < self.len {
            let next = self.source[self.pos];
            let can_start_name = is_tag_name_byte(next);
            let can_start_blade_construct = !self.verbatim
                && !self.php_block
                && !self.php_tag
                && next == b'{'
                && (self.peek_ahead(1) == Some(b'{')
                    || (self.peek_ahead(1) == Some(b'!') && self.peek_ahead(2) == Some(b'!')));

            if !can_start_name
                && !can_start_blade_construct
                && next != b'<'
                && next != b'>'
                && next != b'/'
            {
                self.emit_token(TokenType::Whitespace, start, name_start);
            }
        }

        // Emit tag name and accumulate for rawtext check
        if name_start < self.pos {
            self.current_tag_name
                .extend_from_slice(&self.source[name_start..self.pos]);
            self.emit_token(TokenType::TagName, name_start, self.pos);
        }

        if self.pos >= self.len {
            // EOF. Unclosed tag, emit SyntheticClose.
            self.emit_token(TokenType::SyntheticClose, self.pos, self.pos);
            self.state = State::Data;

            return Ok(());
        }

        let ch = self.source[self.pos];

        if ch == b'<' {
            // Preserve skipped leading whitespace for malformed sequences like "<\t<".
            if start < name_start && name_start == self.pos {
                self.emit_token(TokenType::Whitespace, start, name_start);
            }

            // Check for PHP tag first
            if self.pos + 1 < self.len
                && self.source[self.pos + 1] == b'?'
                && self.try_scan_php_tag()
            {
                // Stay in TagName to continue collecting tag name parts
                self.state = State::TagName;
                self.continued_tag_name = true;

                return Ok(());
            }

            // Not a PHP tag, let's try TSX generic type parameter
            if self.try_scan_tsx_generic_type()? {
                self.state = State::BeforeAttrName;

                return Ok(());
            }

            // A new tag starting before the current tag is closed.
            self.emit_token(TokenType::SyntheticClose, self.pos, self.pos);
            self.scan_tag_open();
        } else if ch == b'>' {
            // End of tag
            self.emit_token(TokenType::GreaterThan, self.pos, self.pos + 1);
            self.pos += 1;
            self.state = self.state_after_tag_close();
        } else if ch == b'/' {
            // Self-closing tag: <br/>
            let slash_start = self.pos;
            self.pos += 1;

            // Skip whitespace before >
            let ws_start = self.pos;
            self.skip_whitespace();

            if self.pos < self.len && self.source[self.pos] == b'>' {
                // Preserve optional whitespace between / and >.
                self.emit_token(TokenType::Slash, slash_start, self.pos);
                self.emit_token(TokenType::GreaterThan, self.pos, self.pos + 1);
                self.pos += 1;
                self.state = State::Data; // Self-closing never enters rawtext
            } else {
                self.emit_token(TokenType::Slash, slash_start, ws_start);

                if self.pos > ws_start {
                    // Preserve skipped whitespace for malformed "/\s+<next>" tails.
                    self.emit_token(TokenType::SyntheticClose, ws_start, ws_start);
                    self.pos = ws_start;
                    self.state = State::Data;
                } else {
                    // Malformed self-closing tag, but continue
                    self.state = State::BeforeAttrName;
                }
            }
        } else {
            // Whitespace after the tag name, or an unexpected character
            self.state = State::BeforeAttrName;
        }

        Ok(())
    }

    /// `scanBeforeAttrName`.
    pub(super) fn scan_before_attr_name(&mut self) -> Result<(), Exhausted> {
        if self.php_block {
            // Scan the PHP block, then resume at the next attribute.
            self.return_state = State::BeforeAttrName;
            return self.scan_data();
        }

        let start = self.pos;

        while self.pos < self.len && is_space(self.source[self.pos]) {
            self.pos += 1;
        }

        // For closing tags, check if the tag is malformed before emitting whitespace
        if self.is_closing_tag {
            if self.pos >= self.len {
                // EOF. SyntheticClose at the whitespace start so whitespace becomes text
                self.emit_token(TokenType::SyntheticClose, start, start);
                self.pos = start;
                self.state = State::Data;

                return Ok(());
            }

            let ch = self.source[self.pos];
            if ch != b'>' && ch != b'/' {
                // Malformed closing tag. SyntheticClose at whitespace start
                self.emit_token(TokenType::SyntheticClose, start, start);
                self.pos = start;
                self.state = State::Data;

                return Ok(());
            }
        }

        if start < self.pos {
            self.emit_token(TokenType::Whitespace, start, self.pos);
        }

        if self.pos >= self.len {
            // EOF. Unclosed tag, emit SyntheticClose
            self.emit_token(TokenType::SyntheticClose, self.pos, self.pos);
            self.state = State::Data;

            return Ok(());
        }

        let ch = self.source[self.pos];

        // Check for Blade directive as attribute: @if($condition)
        if ch == b'@' && !self.verbatim && !self.php_block && !self.php_tag {
            let mut temp_pos = self.pos + 1; // Skip @
            let name_start = temp_pos;

            while temp_pos < self.len {
                let byte = self.source[temp_pos];
                if is_alnum(byte) || byte == b'_' {
                    temp_pos += 1;
                } else {
                    break;
                }
            }

            if temp_pos > name_start {
                let name = &self.source[name_start..temp_pos];
                if self.directives.is_directive(name) && !self.has_modifier_call_at(temp_pos) {
                    self.return_state = State::BeforeAttrName;
                    self.scan_directive();

                    return Ok(());
                }
            }

            // Not a known directive, we'll treat it as the beginning of an attribute.
        }

        // Check for PHP tag first
        if ch == b'<'
            && self.pos + 1 < self.len
            && self.source[self.pos + 1] == b'?'
            && self.try_scan_php_tag()
        {
            self.state = State::BeforeAttrName;

            return Ok(());
        }

        // Check for HTML comment (<!--) in attribute context
        if ch == b'<'
            && self.pos + 3 < self.len
            && self.source[self.pos + 1] == b'!'
            && self.source[self.pos + 2] == b'-'
            && self.source[self.pos + 3] == b'-'
        {
            // SyntheticClose for the incomplete current tag; Data handles the comment
            self.emit_token(TokenType::SyntheticClose, self.pos, self.pos);
            self.state = State::Data;

            return Ok(());
        }

        // Check for TSX generic type parameter: <User> or <{ id: number }>
        if ch == b'<' {
            if self.try_scan_tsx_generic_type()? {
                return Ok(());
            }

            // An entirely new tag (malformed sequence).
            self.emit_token(TokenType::SyntheticClose, self.pos, self.pos);
            self.emit_token(TokenType::LessThan, self.pos, self.pos + 1);
            self.pos += 1;
            self.state = State::TagName;

            return Ok(());
        }

        // Check for JSX shorthand attribute: {enabled} or {...props}
        if ch == b'{'
            && !self.verbatim
            && !self.php_block
            && !self.php_tag
            && self.try_scan_jsx_shorthand_attribute()?
        {
            return Ok(());
        }

        // Check for Blade echo as an attribute or standalone: {{ $attributes }}
        if ch == b'{' && !self.verbatim && !self.php_block && !self.php_tag {
            let saved_pos = self.pos;

            if self.peek_ahead(1) == Some(b'{') {
                // Check for {{--
                if self.peek_ahead(2) == Some(b'-') && self.peek_ahead(3) == Some(b'-') {
                    self.return_state = State::BeforeAttrName;
                    self.scan_blade_comment_start();
                    self.scan_blade_comment_content();

                    return Ok(());
                }
                // Found {{ or {{{
                self.return_state = State::BeforeAttrName;
                self.scan_echo();
                // An echo followed by an attribute name continuation goes on in AttrName
                if self.pos < self.len {
                    let next_ch = self.source[self.pos];
                    if !matches!(
                        next_ch,
                        b' ' | b'\t' | b'\n' | b'\r' | b'>' | b'/' | b'=' | b'<'
                    ) {
                        self.state = State::AttrName;

                        return Ok(());
                    }
                }

                return Ok(());
            } else if self.peek_ahead(1) == Some(b'!') && self.peek_ahead(2) == Some(b'!') {
                // Found {!!
                self.return_state = State::BeforeAttrName;
                self.scan_raw_echo();

                return Ok(());
            }

            // Not a Blade echo. Restore position and continue
            self.pos = saved_pos;
        }

        // Check for XML declaration ending
        if self.in_xml_declaration
            && ch == b'?'
            && self.pos + 1 < self.len
            && self.source[self.pos + 1] == b'>'
        {
            self.emit_token(TokenType::DeclEnd, self.pos, self.pos + 2);
            self.pos += 2;
            self.in_xml_declaration = false;
            self.state = State::Data;

            return Ok(());
        }

        if ch == b'>' {
            // End of tag
            self.emit_token(TokenType::GreaterThan, self.pos, self.pos + 1);
            self.pos += 1;
            self.state = self.state_after_tag_close();
        } else if ch == b'/' {
            // Possible self-closing tag: />
            let slash_start = self.pos;
            self.pos += 1;

            // Skip whitespace before >
            let ws_start = self.pos;
            while self.pos < self.len && is_space(self.source[self.pos]) {
                self.pos += 1;
            }

            if self.pos < self.len && self.source[self.pos] == b'>' {
                self.emit_token(TokenType::Slash, slash_start, self.pos);
                self.emit_token(TokenType::GreaterThan, self.pos, self.pos + 1);
                self.pos += 1;
                // Self-closing never enters rawtext
            } else if self.pos >= self.len {
                self.emit_token(TokenType::Slash, slash_start, ws_start);

                // EOF after /. Preserve skipped whitespace by rewinding when needed.
                if self.pos > ws_start {
                    self.emit_token(TokenType::SyntheticClose, ws_start, ws_start);
                    self.pos = ws_start;
                } else {
                    self.emit_token(TokenType::SyntheticClose, self.pos, self.pos);
                }
            } else {
                self.emit_token(TokenType::Slash, slash_start, ws_start);

                // Malformed self-closing tag (e.g., /\n< instead of />): SyntheticClose, then
                // back to after the slash so whitespace becomes text content
                self.emit_token(TokenType::SyntheticClose, ws_start, ws_start);
                self.pos = ws_start;
            }
            self.state = State::Data;
        } else {
            self.state = State::AttrName;
        }

        Ok(())
    }
}
