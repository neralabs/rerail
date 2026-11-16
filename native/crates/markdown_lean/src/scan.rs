//! tree-sitter-md's external scanner (`markdown/src/scanner.c`), ported line by line: the block
//! structure of a Markdown text is decided here, by a stack of open blocks matched at each line's
//! start, and the parser (`parse`) asks for a token with the set of external tokens its grammar
//! state takes (`valid`), as tree-sitter's parse table gives them.
//!
//! The port keeps the C's names and order, so the two can be read side by side. Two things differ:
//! the lexer is a byte cursor over the text (a character past ASCII is one step, as tree-sitter's
//! lexer reads a code point), and a scan that finds nothing leaves the scanner as it was (tree-sitter
//! restores the state from the last token's, by `deserialize`).

/// The external tokens, in the grammar's order (`TokenType`).
#[allow(dead_code)]
pub mod t {
    pub const LINE_ENDING: u8 = 0;
    pub const SOFT_LINE_ENDING: u8 = 1;
    pub const BLOCK_CLOSE: u8 = 2;
    pub const BLOCK_CONTINUATION: u8 = 3;
    pub const BLOCK_QUOTE_START: u8 = 4;
    pub const INDENTED_CHUNK_START: u8 = 5;
    pub const ATX_H1_MARKER: u8 = 6;
    pub const ATX_H6_MARKER: u8 = 11;
    pub const SETEXT_H1_UNDERLINE: u8 = 12;
    pub const SETEXT_H2_UNDERLINE: u8 = 13;
    pub const THEMATIC_BREAK: u8 = 14;
    pub const LIST_MARKER_MINUS: u8 = 15;
    pub const LIST_MARKER_PLUS: u8 = 16;
    pub const LIST_MARKER_STAR: u8 = 17;
    pub const LIST_MARKER_PARENTHESIS: u8 = 18;
    pub const LIST_MARKER_DOT: u8 = 19;
    pub const LIST_MARKER_MINUS_DONT_INTERRUPT: u8 = 20;
    pub const LIST_MARKER_PLUS_DONT_INTERRUPT: u8 = 21;
    pub const LIST_MARKER_STAR_DONT_INTERRUPT: u8 = 22;
    pub const LIST_MARKER_PARENTHESIS_DONT_INTERRUPT: u8 = 23;
    pub const LIST_MARKER_DOT_DONT_INTERRUPT: u8 = 24;
    pub const FENCED_CODE_BLOCK_START_BACKTICK: u8 = 25;
    pub const FENCED_CODE_BLOCK_START_TILDE: u8 = 26;
    pub const BLANK_LINE_START: u8 = 27;
    pub const FENCED_CODE_BLOCK_END_BACKTICK: u8 = 28;
    pub const FENCED_CODE_BLOCK_END_TILDE: u8 = 29;
    pub const HTML_BLOCK_1_START: u8 = 30;
    pub const HTML_BLOCK_1_END: u8 = 31;
    pub const HTML_BLOCK_2_START: u8 = 32;
    pub const HTML_BLOCK_3_START: u8 = 33;
    pub const HTML_BLOCK_4_START: u8 = 34;
    pub const HTML_BLOCK_5_START: u8 = 35;
    pub const HTML_BLOCK_6_START: u8 = 36;
    pub const HTML_BLOCK_7_START: u8 = 37;
    pub const CLOSE_BLOCK: u8 = 38;
    pub const NO_INDENTED_CHUNK: u8 = 39;
    pub const ERROR: u8 = 40;
    pub const TRIGGER_ERROR: u8 = 41;
    pub const TOKEN_EOF: u8 = 42;
    pub const MINUS_METADATA: u8 = 43;
    pub const PLUS_METADATA: u8 = 44;
    pub const PIPE_TABLE_START: u8 = 45;
    pub const PIPE_TABLE_LINE_ENDING: u8 = 46;
}
use crate::block::Blocks;
use t::*;

/// A set of external tokens, a bit each.
pub type Valid = u64;

pub const fn bit(tok: u8) -> Valid {
    1 << tok
}

#[inline(always)]
fn has(v: Valid, tok: u8) -> bool {
    v & bit(tok) != 0
}

/// `paragraph_interrupt_symbols`.
const PARAGRAPH_INTERRUPT: Valid = bit(BLOCK_QUOTE_START)
    | bit(6)
    | bit(7)
    | bit(8)
    | bit(9)
    | bit(10)
    | bit(11)
    | bit(SETEXT_H1_UNDERLINE)
    | bit(SETEXT_H2_UNDERLINE)
    | bit(THEMATIC_BREAK)
    | bit(LIST_MARKER_MINUS)
    | bit(LIST_MARKER_PLUS)
    | bit(LIST_MARKER_STAR)
    | bit(LIST_MARKER_PARENTHESIS)
    | bit(LIST_MARKER_DOT)
    | bit(FENCED_CODE_BLOCK_START_BACKTICK)
    | bit(FENCED_CODE_BLOCK_START_TILDE)
    | bit(BLANK_LINE_START)
    | bit(HTML_BLOCK_1_START)
    | bit(HTML_BLOCK_2_START)
    | bit(HTML_BLOCK_3_START)
    | bit(HTML_BLOCK_4_START)
    | bit(HTML_BLOCK_5_START)
    | bit(HTML_BLOCK_6_START)
    | bit(PIPE_TABLE_START);

/// `opens_block`.
const OPENS_BLOCK: Valid = bit(BLOCK_QUOTE_START)
    | bit(INDENTED_CHUNK_START)
    | (0x3ff << LIST_MARKER_MINUS)
    | bit(FENCED_CODE_BLOCK_START_BACKTICK)
    | bit(FENCED_CODE_BLOCK_START_TILDE)
    | bit(HTML_BLOCK_1_START)
    | (0x3f << HTML_BLOCK_2_START);

/// A block on the stack (`Block`): its C value.
pub const BLOCK_QUOTE: u8 = 0;
pub const INDENTED_CODE_BLOCK: u8 = 1;
pub const LIST_ITEM: u8 = 2;
pub const LIST_ITEM_MAX_INDENTATION: u8 = 17;
pub const FENCED_CODE_BLOCK: u8 = 18;
pub const ANONYMOUS: u8 = 19;

/// The C's `MAX_OPEN_BLOCKS` (`(1024 - 5) / sizeof(Block)`).
const MAX_OPEN_BLOCKS: usize = (1024 - 5) / 4;

const STATE_MATCHING: u8 = 1;
const STATE_WAS_SOFT_LINE_BREAK: u8 = 2;
const STATE_CLOSE_BLOCK: u8 = 16;

fn is_punctuation(c: u8) -> bool {
    matches!(c, b'!'..=b'/' | b':'..=b'@' | b'['..=b'`' | b'{'..=b'~')
}

/// The scanner's state between tokens (`Scanner`, the part `serialize` writes).
#[derive(Clone, Copy, Debug, Default)]
struct Scalars {
    state: u8,
    matched: u8,
    indentation: u8,
    column: u8,
    fenced_code_block_delimiter_length: u8,
}

/// The scanner's state between tokens (`Scanner::save`).
pub struct Saved {
    v: Scalars,
    open_blocks: Vec<u8>,
    unsure: bool,
}

/// The scanner, and the lexer it reads with.
pub struct Scanner<'s> {
    src: &'s [u8],
    /// The text's byte classes (stage 1), for the scans over a line's rest.
    pub b: Blocks<'s>,
    pub open_blocks: Vec<u8>,
    v: Scalars,
    simulate: bool,
    /// The lexer: where it is, and where the token ends (`mark_end`; `None`: where it is).
    pos: usize,
    end: Option<usize>,
    /// A character the port does not read as the C does (a name's letter past ASCII, where the
    /// C asks `iswalpha`): the parser refuses the text.
    pub unsure: bool,
    /// The token found (`lexer->result_symbol`).
    result: u8,
}

/// A token the scanner found: its kind and end (it starts where the scan did).
#[derive(Clone, Copy, Debug)]
pub struct Found {
    pub tok: u8,
    pub end: usize,
}

impl<'s> Scanner<'s> {
    pub fn new(src: &'s [u8]) -> Self {
        Scanner { src, b: Blocks::of(src), open_blocks: Vec::new(), v: Scalars::default(), simulate: false, pos: 0, end: None, unsure: false, result: 0 }
    }

    // ---- The lexer.

    #[inline(always)]
    fn la(&self) -> u8 {
        // Past the end, a NUL (tree-sitter's lookahead at the end is 0).
        self.src.get(self.pos).copied().unwrap_or(0)
    }

    #[inline(always)]
    fn eof(&self) -> bool {
        self.pos >= self.src.len()
    }

    /// One character on (`lexer->advance`): a character past ASCII is its UTF-8 bytes.
    #[inline(always)]
    fn step(&mut self) {
        let c = self.la();
        self.pos += if c < 0x80 {
            1
        } else if c >= 0xf0 {
            4
        } else if c >= 0xe0 {
            3
        } else {
            2
        };
        self.pos = self.pos.min(self.src.len());
    }

    fn mark_end(&mut self) {
        if !self.simulate {
            self.end = Some(self.pos);
        }
    }

    /// `advance`: one character on, counting a tab to the next stop of 4.
    fn advance(&mut self) -> u8 {
        let mut size = 1;
        if self.la() == b'\t' {
            size = 4 - self.v.column;
            self.v.column = 0;
        } else {
            self.v.column = (self.v.column + 1) % 4;
        }
        self.step();
        size
    }

    /// `advance` over the text up to `stop`, its column kept (stage 1's masks, not a byte at a
    /// time).
    fn skip_to(&mut self, stop: usize) {
        self.v.column = self.b.column_after(self.pos, stop, self.v.column);
        self.pos = stop;
    }

    fn blank(&self) -> bool {
        matches!(self.la(), b' ' | b'\t')
    }

    fn line_end(&self) -> bool {
        matches!(self.la(), b'\n' | b'\r')
    }

    // ---- The entry: a token at `at`, or `None` (the scanner as it was).

    /// The scanner's token at `at`, given the external tokens the parse state takes.
    pub fn scan_at(&mut self, at: usize, valid: Valid) -> Option<Found> {
        let saved = (self.v, self.open_blocks.len());
        self.pos = at;
        self.end = None;
        self.simulate = false;
        let found = self.scan(valid);
        if found {
            let end = self.end.unwrap_or(self.pos);
            Some(Found { tok: self.result, end })
        } else {
            self.v = saved.0;
            debug_assert_eq!(self.open_blocks.len(), saved.1);
            None
        }
    }

    /// The scanner's state between tokens, to go back to (`restore`): a parse tried and dropped.
    pub fn save(&self) -> Saved {
        Saved { v: self.v, open_blocks: self.open_blocks.clone(), unsure: self.unsure }
    }

    /// The state `save` gave.
    pub fn restore(&mut self, s: Saved) {
        self.v = s.v;
        self.open_blocks = s.open_blocks;
        self.unsure = s.unsure;
    }

    fn push_block(&mut self, b: u8) {
        self.open_blocks.push(b);
    }

    fn pop_block(&mut self) -> u8 {
        self.open_blocks.pop().unwrap_or(ANONYMOUS)
    }

    fn size(&self) -> u8 {
        self.open_blocks.len() as u8
    }

    // ---- `match`.

    fn match_block(&mut self, block: u8) -> bool {
        match block {
            INDENTED_CODE_BLOCK => {
                while self.v.indentation < 4 {
                    if self.blank() {
                        self.v.indentation += self.advance();
                    } else {
                        break;
                    }
                }
                if self.v.indentation >= 4 && !self.line_end() {
                    self.v.indentation -= 4;
                    return true;
                }
            }
            LIST_ITEM..=LIST_ITEM_MAX_INDENTATION => {
                let need = block - LIST_ITEM + 2;
                while self.v.indentation < need {
                    if self.blank() {
                        self.v.indentation += self.advance();
                    } else {
                        break;
                    }
                }
                if self.v.indentation >= need {
                    self.v.indentation -= need;
                    return true;
                }
                if self.line_end() {
                    self.v.indentation = 0;
                    return true;
                }
            }
            BLOCK_QUOTE => {
                while self.blank() {
                    self.v.indentation += self.advance();
                }
                if self.la() == b'>' {
                    self.advance();
                    self.v.indentation = 0;
                    if self.blank() {
                        self.v.indentation += self.advance() - 1;
                    }
                    return true;
                }
            }
            FENCED_CODE_BLOCK | ANONYMOUS => return true,
            // A list item's value past `LIST_ITEM_MAX_INDENTATION` is the C's too: no case.
            _ => {}
        }
        false
    }

    // ---- The block starts.

    fn parse_fenced_code_block(&mut self, delimiter: u8, valid: Valid) -> bool {
        let (start, end) = if delimiter == b'`' {
            (FENCED_CODE_BLOCK_START_BACKTICK, FENCED_CODE_BLOCK_END_BACKTICK)
        } else {
            (FENCED_CODE_BLOCK_START_TILDE, FENCED_CODE_BLOCK_END_TILDE)
        };
        if !has(valid, start) && !has(valid, end) {
            return false;
        }
        let mut level: u32 = 0;
        while self.la() == delimiter {
            self.advance();
            level += 1;
        }
        self.mark_end();
        // The C counts in a `uint8_t`.
        let level = (level & 0xff) as u8;
        if has(valid, end) && self.v.indentation < 4 && level >= self.v.fenced_code_block_delimiter_length {
            while self.blank() {
                self.advance();
            }
            let is_eof = self.eof();
            if is_eof {
                self.mark_end();
            }
            if self.line_end() || is_eof {
                self.v.fenced_code_block_delimiter_length = 0;
                self.result = end;
                return true;
            }
        }
        if has(valid, start) && level >= 3 {
            let mut has_backtick = false;
            if delimiter == b'`' {
                // On to the first `` ` `` or the line's end.
                let stop = self.b.tick_or_line_end(self.pos);
                self.skip_to(stop);
                has_backtick = self.la() == b'`';
            }
            if !has_backtick {
                self.result = start;
                if !self.simulate {
                    self.push_block(FENCED_CODE_BLOCK);
                }
                self.v.fenced_code_block_delimiter_length = level;
                self.v.indentation = 0;
                return true;
            }
        }
        false
    }

    /// The list item's block, from its marker's indentation (the C's arithmetic, in `uint8_t`).
    fn list_item(&mut self, mut extra: u8, digits: u8) {
        extra = extra.wrapping_sub(1);
        if extra <= 3 {
            extra = extra.wrapping_add(self.v.indentation);
            self.v.indentation = 0;
        } else {
            std::mem::swap(&mut self.v.indentation, &mut extra);
        }
        if !self.simulate {
            self.push_block(LIST_ITEM.wrapping_add(extra).wrapping_add(digits));
        }
    }

    fn parse_star(&mut self, valid: Valid) -> bool {
        if !has(valid, THEMATIC_BREAK) && !has(valid, LIST_MARKER_STAR) && !has(valid, LIST_MARKER_STAR_DONT_INTERRUPT) {
            return false;
        }
        self.advance();
        self.mark_end();
        let mut star_count = 1usize;
        let mut extra: u8 = 0;
        loop {
            if self.la() == b'*' {
                if star_count == 1 && extra >= 1 && has(valid, LIST_MARKER_STAR) {
                    self.mark_end();
                }
                star_count += 1;
                self.advance();
            } else if self.blank() {
                if star_count == 1 {
                    extra = extra.wrapping_add(self.advance());
                } else {
                    self.advance();
                }
            } else {
                break;
            }
        }
        let line_end = self.line_end();
        let mut dont_interrupt = false;
        if star_count == 1 && line_end {
            extra = 1;
            dont_interrupt = self.v.matched == self.size();
        }
        let thematic_break = star_count >= 3 && line_end;
        let list_marker_star = star_count >= 1 && extra >= 1;
        if has(valid, THEMATIC_BREAK) && thematic_break && self.v.indentation < 4 {
            self.result = THEMATIC_BREAK;
            self.mark_end();
            self.v.indentation = 0;
            return true;
        }
        let marker = if dont_interrupt { LIST_MARKER_STAR_DONT_INTERRUPT } else { LIST_MARKER_STAR };
        if has(valid, marker) && list_marker_star {
            if star_count == 1 {
                self.mark_end();
            }
            self.list_item(extra, 0);
            self.result = marker;
            return true;
        }
        false
    }

    fn parse_thematic_break_underscore(&mut self, valid: Valid) -> bool {
        if !has(valid, THEMATIC_BREAK) {
            return false;
        }
        self.advance();
        self.mark_end();
        let mut count = 1;
        loop {
            if self.la() == b'_' {
                count += 1;
                self.advance();
            } else if self.blank() {
                self.advance();
            } else {
                break;
            }
        }
        if count >= 3 && self.line_end() {
            self.result = THEMATIC_BREAK;
            self.mark_end();
            self.v.indentation = 0;
            return true;
        }
        false
    }

    fn parse_block_quote(&mut self, valid: Valid) -> bool {
        if has(valid, BLOCK_QUOTE_START) {
            self.advance();
            self.v.indentation = 0;
            if self.blank() {
                self.v.indentation = self.v.indentation.wrapping_add(self.advance() - 1);
            }
            self.result = BLOCK_QUOTE_START;
            if !self.simulate {
                self.push_block(BLOCK_QUOTE);
            }
            return true;
        }
        false
    }

    fn parse_atx_heading(&mut self, valid: Valid) -> bool {
        if has(valid, ATX_H1_MARKER) && self.v.indentation <= 3 {
            self.mark_end();
            let mut level: u16 = 0;
            while self.la() == b'#' && level <= 6 {
                self.advance();
                level += 1;
            }
            if level <= 6 && (self.blank() || self.line_end()) {
                self.result = ATX_H1_MARKER + (level as u8 - 1);
                self.v.indentation = 0;
                self.mark_end();
                return true;
            }
        }
        false
    }

    fn parse_setext_underline(&mut self, valid: Valid) -> bool {
        if has(valid, SETEXT_H1_UNDERLINE) && self.v.matched == self.size() {
            self.mark_end();
            while self.la() == b'=' {
                self.advance();
            }
            while self.blank() {
                self.advance();
            }
            if self.line_end() {
                self.result = SETEXT_H1_UNDERLINE;
                self.mark_end();
                return true;
            }
        }
        false
    }

    /// The newline the lexer is at, stepped over (`\n`, `\r` or `\r\n`).
    fn newline(&mut self) {
        if self.la() == b'\r' {
            self.advance();
            if self.la() == b'\n' {
                self.advance();
            }
        } else {
            self.advance();
        }
    }

    /// The rest of a metadata block after its first line of `c`s: a line of exactly three, then a
    /// line end (`*_METADATA`), or the text's end (not metadata).
    fn metadata(&mut self, c: u8, tok: u8) -> bool {
        loop {
            self.newline();
            let mut count = 0;
            while self.la() == c {
                count += 1;
                self.advance();
            }
            if count == 3 {
                while self.blank() {
                    self.advance();
                }
                if self.line_end() {
                    self.newline();
                    self.mark_end();
                    self.result = tok;
                    return true;
                }
            }
            let stop = self.b.line_end(self.pos);
            self.skip_to(stop);
            if self.eof() {
                return false;
            }
        }
    }

    fn parse_plus(&mut self, valid: Valid) -> bool {
        if self.v.indentation <= 3 && (has(valid, LIST_MARKER_PLUS) || has(valid, LIST_MARKER_PLUS_DONT_INTERRUPT) || has(valid, PLUS_METADATA)) {
            self.advance();
            if has(valid, PLUS_METADATA) && self.la() == b'+' {
                self.advance();
                if self.la() != b'+' {
                    return false;
                }
                self.advance();
                while self.blank() {
                    self.advance();
                }
                if !self.line_end() {
                    return false;
                }
                return self.metadata(b'+', PLUS_METADATA);
            } else {
                let mut extra: u8 = 0;
                while self.blank() {
                    extra = extra.wrapping_add(self.advance());
                }
                let mut dont_interrupt = false;
                if self.line_end() {
                    extra = 1;
                    dont_interrupt = true;
                }
                dont_interrupt = dont_interrupt && self.v.matched == self.size();
                let marker = if dont_interrupt { LIST_MARKER_PLUS_DONT_INTERRUPT } else { LIST_MARKER_PLUS };
                if extra >= 1 && has(valid, marker) {
                    self.result = marker;
                    self.list_item(extra, 0);
                    return true;
                }
            }
        }
        false
    }

    fn parse_ordered_list_marker(&mut self, valid: Valid) -> bool {
        if self.v.indentation <= 3
            && (has(valid, LIST_MARKER_PARENTHESIS)
                || has(valid, LIST_MARKER_DOT)
                || has(valid, LIST_MARKER_PARENTHESIS_DONT_INTERRUPT)
                || has(valid, LIST_MARKER_DOT_DONT_INTERRUPT))
        {
            let mut digits: usize = 1;
            self.advance();
            while self.la().is_ascii_digit() {
                digits += 1;
                self.advance();
            }
            let mut dont_interrupt = digits > 1;
            if digits <= 9 {
                let (mut dot, mut parenthesis) = (false, false);
                if self.la() == b'.' {
                    self.advance();
                    dot = true;
                } else if self.la() == b')' {
                    self.advance();
                    parenthesis = true;
                }
                if dot || parenthesis {
                    let mut extra: u8 = 0;
                    while self.blank() {
                        extra = extra.wrapping_add(self.advance());
                    }
                    if self.line_end() {
                        extra = 1;
                        dont_interrupt = true;
                    }
                    dont_interrupt = dont_interrupt && self.v.matched == self.size();
                    let marker = match (dot, dont_interrupt) {
                        (true, true) => LIST_MARKER_DOT_DONT_INTERRUPT,
                        (true, false) => LIST_MARKER_DOT,
                        (false, true) => LIST_MARKER_PARENTHESIS_DONT_INTERRUPT,
                        (false, false) => LIST_MARKER_PARENTHESIS,
                    };
                    if extra >= 1 && has(valid, marker) {
                        self.result = marker;
                        self.list_item(extra, digits as u8);
                        return true;
                    }
                }
            }
        }
        false
    }

    fn parse_minus(&mut self, valid: Valid) -> bool {
        if self.v.indentation <= 3
            && (has(valid, LIST_MARKER_MINUS)
                || has(valid, LIST_MARKER_MINUS_DONT_INTERRUPT)
                || has(valid, SETEXT_H2_UNDERLINE)
                || has(valid, THEMATIC_BREAK)
                || has(valid, MINUS_METADATA))
        {
            self.mark_end();
            let mut whitespace_after_minus = false;
            let mut minus_after_whitespace = false;
            let mut minus_count = 0usize;
            let mut extra: u8 = 0;
            loop {
                if self.la() == b'-' {
                    if minus_count == 1 && extra >= 1 {
                        self.mark_end();
                    }
                    minus_count += 1;
                    self.advance();
                    minus_after_whitespace = whitespace_after_minus;
                } else if self.blank() {
                    if minus_count == 1 {
                        extra = extra.wrapping_add(self.advance());
                    } else {
                        self.advance();
                    }
                    whitespace_after_minus = true;
                } else {
                    break;
                }
            }
            let line_end = self.line_end();
            let mut dont_interrupt = false;
            if minus_count == 1 && line_end {
                extra = 1;
                dont_interrupt = true;
            }
            dont_interrupt = dont_interrupt && self.v.matched == self.size();
            let thematic_break = minus_count >= 3 && line_end;
            let underline = minus_count >= 1 && !minus_after_whitespace && line_end && self.v.matched == self.size();
            let list_marker_minus = minus_count >= 1 && extra >= 1;
            let mut success = false;
            if has(valid, SETEXT_H2_UNDERLINE) && underline {
                self.result = SETEXT_H2_UNDERLINE;
                self.mark_end();
                self.v.indentation = 0;
                success = true;
            } else if has(valid, THEMATIC_BREAK) && thematic_break {
                self.result = THEMATIC_BREAK;
                self.mark_end();
                self.v.indentation = 0;
                success = true;
            } else {
                let marker = if dont_interrupt { LIST_MARKER_MINUS_DONT_INTERRUPT } else { LIST_MARKER_MINUS };
                if has(valid, marker) && list_marker_minus {
                    if minus_count == 1 {
                        self.mark_end();
                    }
                    self.list_item(extra, 0);
                    self.result = marker;
                    return true;
                }
            }
            if minus_count == 3 && !minus_after_whitespace && line_end && has(valid, MINUS_METADATA) {
                // The C goes on from here even after a success above (its result is then
                // replaced where metadata is found).
                if self.metadata(b'-', MINUS_METADATA) {
                    return true;
                }
            }
            if success {
                return true;
            }
        }
        false
    }

    /// `iswalpha` on the lookahead: ASCII letters; a character past ASCII is not read.
    fn alpha(&mut self) -> bool {
        let c = self.la();
        if c >= 0x80 {
            self.unsure = true;
        }
        c.is_ascii_alphabetic()
    }

    fn alnum(&mut self) -> bool {
        let c = self.la();
        if c >= 0x80 {
            self.unsure = true;
        }
        c.is_ascii_alphanumeric()
    }

    fn parse_html_block(&mut self, valid: Valid) -> bool {
        if valid & (0xff << HTML_BLOCK_1_START) == 0 {
            return false;
        }
        self.advance();
        if self.la() == b'?' && has(valid, HTML_BLOCK_3_START) {
            self.advance();
            self.result = HTML_BLOCK_3_START;
            if !self.simulate {
                self.push_block(ANONYMOUS);
            }
            return true;
        }
        if self.la() == b'!' {
            self.advance();
            if self.la() == b'-' {
                self.mark_end();
                let column = self.v.column;
                self.advance();
                if self.la() != b'-' || !has(valid, HTML_BLOCK_2_START) {
                    return false;
                }
                self.advance();
                if self.la() == b'>' || self.la() == b'-' {
                    self.v.column = column;
                } else {
                    self.mark_end();
                }
                self.result = HTML_BLOCK_2_START;
                if !self.simulate {
                    self.push_block(ANONYMOUS);
                }
                return true;
            } else if self.la().is_ascii_uppercase() && has(valid, HTML_BLOCK_4_START) {
                self.advance();
                self.result = HTML_BLOCK_4_START;
                if !self.simulate {
                    self.push_block(ANONYMOUS);
                }
                return true;
            } else if self.la() == b'[' {
                self.advance();
                let mut cdata = true;
                for &c in b"CDATA" {
                    if self.la() != c {
                        cdata = false;
                        break;
                    }
                    self.advance();
                }
                if cdata && self.la() == b'[' && has(valid, HTML_BLOCK_5_START) {
                    self.advance();
                    self.result = HTML_BLOCK_5_START;
                    if !self.simulate {
                        self.push_block(ANONYMOUS);
                    }
                    return true;
                }
                // Otherwise the C goes on to read a tag name from here.
            }
        }
        let starting_slash = self.la() == b'/';
        if starting_slash {
            self.advance();
        }
        let mut name = [0u8; 11];
        let mut name_length = 0usize;
        while self.alpha() {
            if name_length < 10 {
                name[name_length] = self.la().to_ascii_lowercase();
                name_length += 1;
            } else {
                name_length = 12;
            }
            self.advance();
        }
        if name_length == 0 {
            return false;
        }
        let mut tag_closed = false;
        if name_length < 11 {
            let name = &name[..name_length];
            let next_symbol_valid = self.blank() || self.line_end() || self.la() == b'>';
            if next_symbol_valid && HTML_TAG_NAMES_RULE_1.contains(&name) {
                if starting_slash {
                    if has(valid, HTML_BLOCK_1_END) {
                        self.result = HTML_BLOCK_1_END;
                        return true;
                    }
                } else if has(valid, HTML_BLOCK_1_START) {
                    self.result = HTML_BLOCK_1_START;
                    if !self.simulate {
                        self.push_block(ANONYMOUS);
                    }
                    return true;
                }
            }
            if !next_symbol_valid && self.la() == b'/' {
                self.advance();
                if self.la() == b'>' {
                    self.advance();
                    tag_closed = true;
                }
            }
            if (next_symbol_valid || tag_closed) && HTML_TAG_NAMES_RULE_7.contains(&name) && has(valid, HTML_BLOCK_6_START) {
                self.result = HTML_BLOCK_6_START;
                if !self.simulate {
                    self.push_block(ANONYMOUS);
                }
                return true;
            }
        }
        if !has(valid, HTML_BLOCK_7_START) {
            return false;
        }
        if !tag_closed {
            while self.alnum() || self.la() == b'-' {
                self.advance();
            }
            if !starting_slash {
                let mut had_whitespace = false;
                loop {
                    while self.blank() {
                        had_whitespace = true;
                        self.advance();
                    }
                    if self.la() == b'/' {
                        self.advance();
                        break;
                    }
                    if self.la() == b'>' {
                        break;
                    }
                    if !had_whitespace {
                        return false;
                    }
                    if !self.alpha() && self.la() != b'_' && self.la() != b':' {
                        return false;
                    }
                    had_whitespace = false;
                    self.advance();
                    while self.alnum() || matches!(self.la(), b'_' | b'.' | b':' | b'-') {
                        self.advance();
                    }
                    while self.blank() {
                        had_whitespace = true;
                        self.advance();
                    }
                    if self.la() == b'=' {
                        self.advance();
                        had_whitespace = false;
                        while self.blank() {
                            self.advance();
                        }
                        if self.la() == b'\'' || self.la() == b'"' {
                            let delimiter = self.la();
                            self.advance();
                            while self.la() != delimiter && !self.line_end() && !self.eof() {
                                self.advance();
                            }
                            if self.la() != delimiter {
                                return false;
                            }
                            self.advance();
                        } else {
                            let mut had_one = false;
                            while !matches!(self.la(), b' ' | b'\t' | b'"' | b'\'' | b'=' | b'<' | b'>' | b'`' | b'\n' | b'\r') && !self.eof() {
                                self.advance();
                                had_one = true;
                            }
                            if !had_one {
                                return false;
                            }
                        }
                    }
                }
            } else {
                while self.blank() {
                    self.advance();
                }
            }
            if self.la() != b'>' {
                return false;
            }
            self.advance();
        }
        while self.blank() {
            self.advance();
        }
        if self.line_end() {
            self.result = HTML_BLOCK_7_START;
            if !self.simulate {
                self.push_block(ANONYMOUS);
            }
            return true;
        }
        false
    }

    fn parse_pipe_table(&mut self) -> bool {
        self.mark_end();
        let mut cell_count = 0usize;
        let mut starting_pipe = false;
        let mut ending_pipe = false;
        let empty = true;
        if self.la() == b'|' {
            starting_pipe = true;
            self.advance();
        }
        while !self.line_end() && !self.eof() {
            if self.la() == b'|' {
                cell_count += 1;
                ending_pipe = true;
                self.advance();
            } else {
                if !self.blank() {
                    ending_pipe = false;
                }
                if self.la() == b'\\' {
                    self.advance();
                    if is_punctuation(self.la()) {
                        self.advance();
                    }
                } else {
                    self.advance();
                }
            }
        }
        if empty && cell_count == 0 && !(starting_pipe && ending_pipe) {
            return false;
        }
        if !ending_pipe {
            cell_count += 1;
        }
        if self.la() == b'\n' {
            self.advance();
        } else if self.la() == b'\r' {
            self.advance();
            if self.la() == b'\n' {
                self.advance();
            }
        } else {
            return false;
        }
        self.v.indentation = 0;
        self.v.column = 0;
        while self.blank() {
            self.v.indentation = self.v.indentation.wrapping_add(self.advance());
        }
        self.simulate = true;
        let mut matched_temp = 0usize;
        while matched_temp < self.open_blocks.len() {
            let b = self.open_blocks[matched_temp];
            if self.match_block(b) {
                matched_temp += 1;
            } else {
                return false;
            }
        }
        let mut delimiter_cell_count = 0usize;
        if self.la() == b'|' {
            self.advance();
        }
        loop {
            while self.blank() {
                self.advance();
            }
            if self.la() == b'|' {
                delimiter_cell_count += 1;
                self.advance();
                continue;
            }
            if self.la() == b':' {
                self.advance();
                if self.la() != b'-' {
                    return false;
                }
            }
            let mut had_one_minus = false;
            while self.la() == b'-' {
                had_one_minus = true;
                self.advance();
            }
            if had_one_minus {
                delimiter_cell_count += 1;
            }
            if self.la() == b':' {
                if !had_one_minus {
                    return false;
                }
                self.advance();
            }
            while self.blank() {
                self.advance();
            }
            if self.la() == b'|' {
                if !had_one_minus {
                    delimiter_cell_count += 1;
                }
                self.advance();
                continue;
            }
            if !self.line_end() {
                return false;
            }
            break;
        }
        if cell_count != delimiter_cell_count {
            return false;
        }
        self.result = PIPE_TABLE_START;
        true
    }

    // ---- `scan`.

    fn scan(&mut self, mut valid: Valid) -> bool {
        if self.open_blocks.len() >= MAX_OPEN_BLOCKS {
            valid &= !OPENS_BLOCK;
        }
        if has(valid, TRIGGER_ERROR) {
            self.result = ERROR;
            return true;
        }
        if has(valid, CLOSE_BLOCK) {
            self.v.state |= STATE_CLOSE_BLOCK;
            self.result = CLOSE_BLOCK;
            return true;
        }
        if self.eof() {
            if has(valid, TOKEN_EOF) {
                self.result = TOKEN_EOF;
                return true;
            }
            if !self.open_blocks.is_empty() {
                self.result = BLOCK_CLOSE;
                if !self.simulate {
                    self.pop_block();
                }
                return true;
            }
            return false;
        }
        if self.v.state & STATE_MATCHING == 0 {
            while self.blank() {
                let a = self.advance();
                self.v.indentation = self.v.indentation.wrapping_add(a);
            }
            if has(valid, INDENTED_CHUNK_START) && !has(valid, NO_INDENTED_CHUNK) && self.v.indentation >= 4 && !self.line_end() {
                self.result = INDENTED_CHUNK_START;
                if !self.simulate {
                    self.push_block(INDENTED_CODE_BLOCK);
                }
                self.v.indentation -= 4;
                return true;
            }
            match self.la() {
                b'\r' | b'\n' => {
                    if has(valid, BLANK_LINE_START) {
                        self.result = BLANK_LINE_START;
                        return true;
                    }
                }
                b'`' => return self.parse_fenced_code_block(b'`', valid),
                b'~' => return self.parse_fenced_code_block(b'~', valid),
                b'*' => return self.parse_star(valid),
                b'_' => return self.parse_thematic_break_underscore(valid),
                b'>' => return self.parse_block_quote(valid),
                b'#' => return self.parse_atx_heading(valid),
                b'=' => return self.parse_setext_underline(valid),
                b'+' => return self.parse_plus(valid),
                b'0'..=b'9' => return self.parse_ordered_list_marker(valid),
                b'-' => return self.parse_minus(valid),
                b'<' => return self.parse_html_block(valid),
                _ => {}
            }
            if !self.line_end() && has(valid, PIPE_TABLE_START) {
                return self.parse_pipe_table();
            }
        } else {
            let mut partial_success = false;
            while self.v.matched < self.size() {
                if self.v.matched == self.size() - 1 && self.v.state & STATE_CLOSE_BLOCK != 0 {
                    if !partial_success {
                        self.v.state &= !STATE_CLOSE_BLOCK;
                    }
                    break;
                }
                let b = self.open_blocks[usize::from(self.v.matched)];
                if self.match_block(b) {
                    partial_success = true;
                    self.v.matched += 1;
                } else {
                    if self.v.state & STATE_WAS_SOFT_LINE_BREAK != 0 {
                        self.v.state &= !STATE_MATCHING;
                    }
                    break;
                }
            }
            if partial_success {
                if self.v.matched == self.size() {
                    self.v.state &= !STATE_MATCHING;
                }
                self.result = BLOCK_CONTINUATION;
                return true;
            }
            if self.v.state & STATE_WAS_SOFT_LINE_BREAK == 0 {
                self.result = BLOCK_CLOSE;
                self.pop_block();
                if self.v.matched == self.size() {
                    self.v.state &= !STATE_MATCHING;
                }
                return true;
            }
        }
        if (has(valid, LINE_ENDING) || has(valid, SOFT_LINE_ENDING) || has(valid, PIPE_TABLE_LINE_ENDING)) && self.line_end() {
            self.newline();
            self.v.indentation = 0;
            self.v.column = 0;
            if self.v.state & STATE_CLOSE_BLOCK == 0 && (has(valid, SOFT_LINE_ENDING) || has(valid, PIPE_TABLE_LINE_ENDING)) {
                // `lexer->mark_end` (not the scanner's: it holds even when simulating).
                self.end = Some(self.pos);
                while self.blank() {
                    let a = self.advance();
                    self.v.indentation = self.v.indentation.wrapping_add(a);
                }
                self.simulate = true;
                let matched_temp = self.v.matched;
                self.v.matched = 0;
                let mut one_will_be_matched = false;
                while self.v.matched < self.size() {
                    let b = self.open_blocks[usize::from(self.v.matched)];
                    if self.match_block(b) {
                        self.v.matched += 1;
                        one_will_be_matched = true;
                    } else {
                        break;
                    }
                }
                let all_will_be_matched = self.v.matched == self.size();
                // The nested scan's result symbol is the C's `lexer->result_symbol`, which the
                // branches below set again.
                if !self.eof() && !self.scan(PARAGRAPH_INTERRUPT) {
                    self.v.matched = 0;
                    self.v.indentation = 0;
                    self.v.column = 0;
                    if one_will_be_matched {
                        self.v.state |= STATE_MATCHING;
                    } else {
                        self.v.state &= !STATE_MATCHING;
                    }
                    if has(valid, PIPE_TABLE_LINE_ENDING) {
                        if all_will_be_matched {
                            self.result = PIPE_TABLE_LINE_ENDING;
                            return true;
                        }
                    } else {
                        self.result = SOFT_LINE_ENDING;
                        self.v.state |= STATE_WAS_SOFT_LINE_BREAK;
                        return true;
                    }
                } else {
                    self.v.matched = matched_temp;
                }
                self.v.indentation = 0;
                self.v.column = 0;
            }
            if has(valid, LINE_ENDING) {
                self.v.matched = 0;
                if !self.open_blocks.is_empty() {
                    self.v.state |= STATE_MATCHING;
                } else {
                    self.v.state &= !STATE_MATCHING;
                }
                self.v.state &= !STATE_WAS_SOFT_LINE_BREAK;
                self.result = LINE_ENDING;
                return true;
            }
        }
        false
    }
}

const HTML_TAG_NAMES_RULE_1: &[&[u8]] = &[b"pre", b"script", b"style"];

const HTML_TAG_NAMES_RULE_7: &[&[u8]] = &[
    b"address", b"article", b"aside", b"base", b"basefont", b"blockquote", b"body", b"caption", b"center", b"col", b"colgroup", b"dd",
    b"details", b"dialog", b"dir", b"div", b"dl", b"dt", b"fieldset", b"figcaption", b"figure", b"footer", b"form", b"frame",
    b"frameset", b"h1", b"h2", b"h3", b"h4", b"h5", b"h6", b"head", b"header", b"hr", b"html", b"iframe", b"legend", b"li", b"link",
    b"main", b"menu", b"menuitem", b"nav", b"noframes", b"ol", b"optgroup", b"option", b"p", b"param", b"section", b"source",
    b"summary", b"table", b"tbody", b"td", b"tfoot", b"th", b"thead", b"title", b"tr", b"track", b"ul",
];
