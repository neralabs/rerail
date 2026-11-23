//! `Lexer/Concerns/JsxScanner.php`. These scans look ahead and restore their position when they
//! fail, so each spends the distance it scanned.
use super::super::Exhausted;
use super::super::scan;
use super::super::token::{State, TokenType};
use super::Lexer;

/// A `strcspn` mask as a byte table.
const fn mask(bytes: &[u8]) -> [bool; 256] {
    let mut table = [false; 256];
    let mut i = 0;
    while i < bytes.len() {
        table[bytes[i] as usize] = true;
        i += 1;
    }
    table
}

/// `tryScanTsxGenericType`'s mask.
const TSX_GENERIC_MASK: [bool; 256] = mask(b"<>\"'`@\n\r");

/// `scanBalancedJsLike`'s mask.
const BALANCED_JS_MASK: [bool; 256] = mask(b"{}()\"'`/@\n\r");

impl Lexer<'_> {
    /// Spends the bytes a lookahead scanned from `from` to the current position.
    fn spend_scanned(&mut self, from: usize) -> Result<(), Exhausted> {
        self.budget.spend(self.pos.saturating_sub(from) as u64)
    }

    /// `tryScanJsxShorthandAttribute`: `{enabled}` or `{...props}`.
    pub(super) fn try_scan_jsx_shorthand_attribute(&mut self) -> Result<bool, Exhausted> {
        if self.pos >= self.len || self.source[self.pos] != b'{' {
            return Ok(false);
        }

        // Make sure it's not an echo.
        if let Some(b'{' | b'!') = self.at(self.pos + 1) {
            return Ok(false);
        }

        let start = self.pos;
        let original_pos = self.pos;
        self.pos += 1;

        let mut fast_scan_pos = self.pos;
        while fast_scan_pos < self.len {
            let byte = self.source[fast_scan_pos];

            if byte == b'}' {
                self.budget.spend((fast_scan_pos - start) as u64)?;
                self.pos = fast_scan_pos + 1;
                self.emit_token(TokenType::JsxShorthandAttribute, start, self.pos);
                self.state = State::BeforeAttrName;

                return Ok(true);
            }

            if matches!(byte, b'{' | b'\'' | b'"' | b'`' | b'/') {
                break; // complex case
            }

            if matches!(byte, b'\n' | b'\r' | b'@') {
                self.budget.spend((fast_scan_pos - start) as u64)?;
                self.pos = original_pos;

                return Ok(false);
            }

            fast_scan_pos += 1;
        }
        self.budget.spend((fast_scan_pos - start) as u64)?;

        let ok = self.scan_balanced_js_like(
            start,
            TokenType::JsxShorthandAttribute,
            original_pos,
            1,
            0,
            false,
            true,
            true,
            true,
        )?;

        if ok {
            self.state = State::BeforeAttrName;
        }

        Ok(ok)
    }

    /// `scanJsxAttributeValue`: `data={expression}` or `data=({expression})`.
    pub(super) fn scan_jsx_attribute_value(&mut self) -> Result<bool, Exhausted> {
        let start = self.pos;
        let original_pos = self.pos;

        let starts_with_paren = self.at(self.pos) == Some(b'(');
        self.pos += 1;

        let brace_depth: isize = if starts_with_paren { 0 } else { 1 };
        let paren_depth: isize = if starts_with_paren { 1 } else { 0 };

        // Simple {expr} with no nesting and no strings/templates/comments
        if brace_depth == 1 && paren_depth == 0 {
            let mut fast_scan_pos = self.pos;
            while fast_scan_pos < self.len {
                let byte = self.source[fast_scan_pos];

                if byte == b'}' {
                    self.budget.spend((fast_scan_pos - start) as u64)?;
                    self.pos = fast_scan_pos + 1;
                    self.emit_token(TokenType::JsxAttributeValue, start, self.pos);

                    return Ok(true);
                }

                if matches!(byte, b'{' | b'(' | b'\'' | b'"' | b'`' | b'/') {
                    break;
                }

                if matches!(byte, b'\n' | b'\r' | b'@') {
                    self.budget.spend((fast_scan_pos - start) as u64)?;
                    self.pos = original_pos;

                    return Ok(false);
                }

                fast_scan_pos += 1;
            }
            self.budget.spend((fast_scan_pos - start) as u64)?;
        }

        self.scan_balanced_js_like(
            start,
            TokenType::JsxAttributeValue,
            original_pos,
            brace_depth,
            paren_depth,
            true,
            true,
            true,
            false,
        )
    }

    /// `tryScanTsxGenericType`: `<User>` or `<{ id: number }>`.
    pub(super) fn try_scan_tsx_generic_type(&mut self) -> Result<bool, Exhausted> {
        if self.pos >= self.len || self.source[self.pos] != b'<' {
            return Ok(false);
        }

        // Check if we are starting a PHP tag.
        if self.at(self.pos + 1) == Some(b'?') {
            return Ok(false);
        }

        let start = self.pos;
        let original_pos = self.pos;
        self.pos += 1; // Skip opening <

        // Validate the next character looks like a generic start
        if self.pos < self.len {
            let next_byte = self.source[self.pos];

            let is_pascal_case_tag = self
                .current_tag_name
                .first()
                .is_some_and(|first| first.is_ascii_uppercase());

            let mut looks_like_generic =
                next_byte == b'{' || next_byte.is_ascii_uppercase() || next_byte == b'_';

            if is_pascal_case_tag {
                looks_like_generic = looks_like_generic
                    || next_byte.is_ascii_lowercase()
                    || next_byte.is_ascii_digit()
                    || matches!(next_byte, b'[' | b'"' | b'\'' | b'(' | b' ' | b'\t');
            }

            if !looks_like_generic {
                self.pos = original_pos;

                return Ok(false);
            }
        }

        let mut depth = 1usize;

        loop {
            if self.pos >= self.len {
                // EOF. Invalid generic, restore
                self.spend_scanned(start)?;
                self.pos = original_pos;

                return Ok(false);
            }

            let next_pos = self.next_interesting_pos(&TSX_GENERIC_MASK);
            if next_pos >= self.len {
                self.pos = next_pos;
                self.spend_scanned(start)?;
                self.pos = original_pos;

                return Ok(false);
            }

            self.pos = next_pos;
            let byte = self.source[self.pos];

            if byte == b'<' {
                depth += 1;
                self.pos += 1;

                continue;
            }

            if byte == b'>' {
                // Arrow function => should not close generics
                if self.pos > 0 && self.source[self.pos - 1] == b'=' {
                    self.pos += 1;

                    continue;
                }

                depth -= 1;
                self.pos += 1;
                if depth == 0 {
                    self.spend_scanned(start)?;
                    self.emit_token(TokenType::TsxGenericType, start, self.pos);

                    return Ok(true);
                }

                continue;
            }

            if matches!(byte, b'\n' | b'\r' | b'@') {
                self.spend_scanned(start)?;
                self.pos = original_pos;

                return Ok(false);
            }

            if byte == b'"' || byte == b'\'' {
                self.pos += 1;
                scan::skip_quoted_string(self.source, &mut self.pos, self.len, byte);

                continue;
            }

            if byte == b'`' {
                self.pos += 1;
                scan::skip_template_literal(self.source, &mut self.pos, self.len);

                continue;
            }

            self.pos += 1;
        }
    }

    /// `nextInterestingPos`: the next byte in `mask` at or after the position, or the length.
    fn next_interesting_pos(&self, mask: &[bool; 256]) -> usize {
        if self.pos >= self.len {
            return self.len;
        }

        self.source[self.pos..self.len]
            .iter()
            .position(|&byte| mask[byte as usize])
            .map_or(self.len, |skip| self.pos + skip)
    }

    /// `scanBalancedJsLike`: the balanced scanner for JSX-ish content; restores `original_pos`
    /// and answers `false` on an abort.
    #[allow(clippy::too_many_arguments)]
    fn scan_balanced_js_like(
        &mut self,
        start: usize,
        token_type: TokenType,
        original_pos: usize,
        mut brace_depth: isize,
        mut paren_depth: isize,
        track_parens: bool,
        allow_comments: bool,
        abort_on_directive: bool,
        abort_on_newline: bool,
    ) -> Result<bool, Exhausted> {
        let from = self.pos;
        loop {
            if self.pos >= self.len {
                // EOF: emit what we have
                self.spend_scanned(from)?;
                self.emit_token(token_type, start, self.pos);

                return Ok(true);
            }

            let next_pos = self.next_interesting_pos(&BALANCED_JS_MASK);
            if next_pos >= self.len {
                self.pos = self.len;
                self.spend_scanned(from)?;
                self.emit_token(token_type, start, self.pos);

                return Ok(true);
            }

            self.pos = next_pos;
            let byte = self.source[self.pos];

            if byte == b'{' {
                brace_depth += 1;
                self.pos += 1;

                continue;
            }

            if byte == b'}' {
                brace_depth -= 1;
                self.pos += 1;
                if brace_depth == 0 && (!track_parens || paren_depth == 0) {
                    self.spend_scanned(from)?;
                    self.emit_token(token_type, start, self.pos);

                    return Ok(true);
                }

                continue;
            }

            if track_parens && byte == b'(' {
                paren_depth += 1;
                self.pos += 1;

                continue;
            }

            if track_parens && byte == b')' {
                paren_depth -= 1;
                self.pos += 1;
                if brace_depth == 0 && paren_depth == 0 {
                    self.spend_scanned(from)?;
                    self.emit_token(token_type, start, self.pos);

                    return Ok(true);
                }

                continue;
            }

            if byte == b'"' || byte == b'\'' {
                self.pos += 1;
                scan::skip_quoted_string(self.source, &mut self.pos, self.len, byte);

                continue;
            }

            if byte == b'`' {
                self.pos += 1;
                scan::skip_template_literal(self.source, &mut self.pos, self.len);

                continue;
            }

            if byte == b'/' {
                if !allow_comments {
                    self.pos += 1;

                    continue;
                }

                if self.pos + 1 < self.len {
                    let next = self.source[self.pos + 1];
                    if next == b'/' {
                        self.pos += 2;
                        scan::skip_line_comment(self.source, &mut self.pos, self.len);

                        continue;
                    }
                    if next == b'*' {
                        self.pos += 2;
                        scan::skip_block_comment(self.source, &mut self.pos, self.len);

                        continue;
                    }
                }

                self.pos += 1;

                continue;
            }

            if abort_on_directive && byte == b'@' {
                self.spend_scanned(from)?;
                self.pos = original_pos;

                return Ok(false);
            }

            if abort_on_newline && (byte == b'\n' || byte == b'\r') {
                self.spend_scanned(from)?;
                self.pos = original_pos;

                return Ok(false);
            }

            self.pos += 1;
        }
    }
}
