//! tree-sitter-scala's external scanner (`src/scanner.c` of tree-sitter-scala 0.26.2), ported line
//! for line: the indentation stack, automatic semicolons, indents and outdents (a comma's outdent,
//! a same-width `case` region), the control-tail gate, `end` markers, soft modifiers, the
//! fewer-braces colon, postfix operators, operators ending in `/`, and block comments at layout
//! points. Strings' pieces are read by the parser (their states take nothing else).
//!
//! tree-sitter runs the scanner before every token, with the external tokens the parse state takes
//! (`V`), and keeps its state only from the last external token returned: a scan that returns
//! nothing changes nothing. The parser does the same (`Scanner::scan` on a copy, kept where the
//! token is read), at each point its state takes a layout token.

/// The external tokens a parse state takes: a bit each.
pub type V = u64;

#[allow(dead_code)]
pub mod v {
    use super::V;
    pub const SEMI: V = 1 << 0;
    pub const INDENT: V = 1 << 1;
    pub const OUTDENT: V = 1 << 2;
    pub const COMMA_OUTDENT: V = 1 << 3;
    pub const BLOCK_COMMENT: V = 1 << 4;
    pub const SUPPRESS_BLOCK_COMMENT: V = 1 << 5;
    pub const ELSE: V = 1 << 6;
    pub const CATCH: V = 1 << 7;
    pub const FINALLY: V = 1 << 8;
    pub const EXTENDS: V = 1 << 9;
    pub const DERIVES: V = 1 << 10;
    pub const WITH: V = 1 << 11;
    pub const USES: V = 1 << 12;
    pub const COLON_EOL: V = 1 << 13;
    pub const POSTFIX_OP: V = 1 << 14;
    pub const POSTFIX_STAR: V = 1 << 15;
    pub const FLOAT_SEP: V = 1 << 16;
    pub const END_KW: V = 1 << 17;
    pub const GATE: V = 1 << 18;
    pub const XML: V = 1 << 19;
    pub const OP_OR: V = 1 << 20;
    pub const OP_XOR: V = 1 << 21;
    pub const OP_AND: V = 1 << 22;
    pub const OP_EQ: V = 1 << 23;
    pub const OP_REL: V = 1 << 24;
    pub const OP_COLON: V = 1 << 25;
    pub const OP_ADD: V = 1 << 26;
    pub const OP_MUL: V = 1 << 27;
    pub const OP_OTHER: V = 1 << 28;
    pub const OP_NAME: V = 1 << 29;
    pub const ERASED: V = 1 << 30;
    pub const OPEN: V = 1 << 31;
    pub const OPAQUE: V = 1 << 32;
    pub const INFIX: V = 1 << 33;
    pub const TRACKED: V = 1 << 34;
    pub const TRANSPARENT: V = 1 << 35;
    pub const INLINE: V = 1 << 36;
    pub const INTO: V = 1 << 37;
    pub const UPDATE: V = 1 << 38;
    pub const CONSUME: V = 1 << 39;
    pub const STRING: V = 1 << 40;
    /// Every operator class: where an infix operator may follow.
    pub const OPS: V = OP_OR | OP_XOR | OP_AND | OP_EQ | OP_REL | OP_COLON | OP_ADD | OP_MUL | OP_OTHER;
    /// Every soft modifier.
    pub const MODS: V = ERASED | OPEN | OPAQUE | INFIX | TRACKED | TRANSPARENT | INLINE | INTO | UPDATE | CONSUME;
}

/// An external token the scanner returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ext {
    Semi,
    Indent,
    Outdent,
    CommaOutdent,
    BlockComment,
    Gate,
    EndKw,
    ColonEol,
    PostfixOp,
    PostfixStar,
    FloatSep,
    Xml,
    /// An operator ending in `/`, of the class (a `v::OP_…` bit, or `v::OP_NAME`).
    Op(V),
    /// A soft modifier (its `v::…` bit).
    Modifier(V),
}

const CASE_INDENT_FLAG: i16 = 0x4000;

#[inline]
fn indent_width(entry: i16) -> i16 {
    if entry == -1 { -1 } else { entry & 0x3FFF }
}

#[inline]
fn at_case_region_width(prev: i16, width: i16) -> bool {
    prev != -1 && (prev & CASE_INDENT_FLAG) != 0 && width == indent_width(prev)
}

/// The scanner's state, as tree-sitter serializes it after each external token.
#[derive(Clone, Debug)]
pub struct Scanner {
    pub indents: Vec<i16>,
    last_indentation_size: i16,
    last_newline_count: i16,
    last_column: i16,
    last_char: i16,
    after_colon_eol: bool,
}

impl Default for Scanner {
    fn default() -> Self {
        Scanner { indents: Vec::new(), last_indentation_size: -1, last_newline_count: 0, last_column: -1, last_char: 0, after_colon_eol: false }
    }
}

/// The scanner's view of the text: tree-sitter's lexer (a lookahead character, `advance`, `skip`,
/// `mark_end`, `get_column`).
struct Lx<'a> {
    src: &'a [u8],
    i: usize,
    /// Where the token starts: past the last `skip`.
    start: usize,
    end: Option<usize>,
}

impl Lx<'_> {
    #[inline]
    fn la(&self) -> i32 {
        match self.src.get(self.i) {
            None => 0,
            Some(&b) if b < 0x80 => i32::from(b),
            Some(_) => crate::lex::decode(self.src, self.i).map_or(0xfffd, |(c, _)| c as i32),
        }
    }
    #[inline]
    fn eof(&self) -> bool {
        self.i >= self.src.len()
    }
    #[inline]
    fn advance(&mut self) {
        if let Some(&b) = self.src.get(self.i) {
            self.i += if b < 0x80 { 1 } else { crate::lex::decode(self.src, self.i).map_or(1, |(_, n)| n) };
        }
    }
    #[inline]
    fn skip(&mut self) {
        self.advance();
        self.start = self.i;
    }
    #[inline]
    fn mark_end(&mut self) {
        self.end = Some(self.i);
    }
    /// The column: characters from the line's start.
    fn column(&self) -> i16 {
        let line = self.src[..self.i].iter().rposition(|&b| b == b'\n').map_or(0, |p| p + 1);
        self.src[line..self.i].iter().filter(|&&b| b & 0xc0 != 0x80).count() as i16
    }
}

#[inline]
fn is_space(c: i32) -> bool {
    c == b' ' as i32 || (b'\t' as i32..=b'\r' as i32).contains(&c)
}

#[inline]
fn is_alpha(c: i32) -> bool {
    (b'a' as i32..=b'z' as i32).contains(&c) || (b'A' as i32..=b'Z' as i32).contains(&c)
}

#[inline]
fn is_alnum(c: i32) -> bool {
    is_alpha(c) || (b'0' as i32..=b'9' as i32).contains(&c)
}

fn advance_past_blanks(l: &mut Lx) -> bool {
    let mut found = false;
    while l.la() == b' ' as i32 || l.la() == b'\t' as i32 {
        l.advance();
        found = true;
    }
    found
}

fn is_op_char(c: i32) -> bool {
    matches!(u8::try_from(c), Ok(b'!' | b'#' | b'%' | b'&' | b'*' | b'+' | b'-' | b'<' | b'=' | b'>' | b'?' | b'@' | b'\\' | b'^' | b'|' | b'~' | b':'))
}

fn op_left_class(first: i32) -> V {
    match u8::try_from(first) {
        Ok(b'|') => v::OP_OR,
        Ok(b'^') => v::OP_XOR,
        Ok(b'&') => v::OP_AND,
        Ok(b'=' | b'!') => v::OP_EQ,
        Ok(b'<' | b'>') => v::OP_REL,
        Ok(b':') => v::OP_COLON,
        Ok(b'+' | b'-') => v::OP_ADD,
        Ok(b'*' | b'%') => v::OP_MUL,
        _ => v::OP_OTHER,
    }
}

fn is_xml_name_start(c: i32) -> bool {
    is_alpha(c) || c == b'_' as i32 || c > 127
}

fn is_close_or_separator(c: i32) -> bool {
    matches!(u8::try_from(c), Ok(b')' | b']' | b'}' | b',' | b';'))
}

fn consume_block_comment_body_ex(l: &mut Lx, stop_at_newline: bool) -> bool {
    let mut depth = 1u32;
    while depth > 0 {
        if l.eof() || (stop_at_newline && l.la() == b'\n' as i32) {
            return false;
        }
        if l.la() == b'/' as i32 {
            l.advance();
            if l.la() == b'*' as i32 {
                l.advance();
                depth += 1;
            }
        } else if l.la() == b'*' as i32 {
            l.advance();
            if l.la() == b'/' as i32 {
                l.advance();
                depth -= 1;
            }
        } else {
            l.advance();
        }
    }
    true
}

fn consume_block_comment_body(l: &mut Lx) {
    consume_block_comment_body_ex(l, false);
}

fn consume_block_comment_body_on_line(l: &mut Lx) -> bool {
    consume_block_comment_body_ex(l, true)
}

fn scan_word(l: &mut Lx, word: &[u8]) -> bool {
    for &c in word {
        if l.la() != i32::from(c) {
            return false;
        }
        l.advance();
    }
    !(is_alnum(l.la()) || l.la() == b'_' as i32 || l.la() == b'$' as i32)
}

/// One identifier-like word (`[A-Za-z0-9_$]*`); `None` where it cannot be one of the keywords
/// (longer than `cap - 1`). The whole word is consumed.
fn read_word(l: &mut Lx, cap: usize) -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    let mut not_keyword = false;
    while is_alnum(l.la()) || l.la() == b'_' as i32 || l.la() == b'$' as i32 {
        if buf.len() >= cap - 1 {
            not_keyword = true;
        } else {
            buf.push(l.la() as u8);
        }
        l.advance();
    }
    if not_keyword { None } else { Some(buf) }
}

fn word_in(word: &Option<Vec<u8>>, words: &[&[u8]]) -> bool {
    word.as_ref().is_some_and(|w| !w.is_empty() && words.contains(&w.as_slice()))
}

fn skip_blanks_and_block_comments(l: &mut Lx) {
    loop {
        advance_past_blanks(l);
        if l.la() != b'/' as i32 {
            return;
        }
        l.advance();
        if l.la() != b'*' as i32 {
            return;
        }
        l.advance();
        consume_block_comment_body(l);
    }
}

fn name_follows_word(l: &mut Lx) -> bool {
    skip_blanks_and_block_comments(l);
    let c = l.la();
    is_alpha(c) || c == b'_' as i32 || c == b'$' as i32 || c == b'`' as i32 || c > 127
}

fn word_is_expression_tail(l: &mut Lx) -> bool {
    let w = read_word(l, 8);
    word_in(&w, &[b"match", b"catch", b"finally", b"else", b"then", b"do", b"yield", b"while", b"with", b"extends"])
}

fn modifier_name_follows(l: &mut Lx) -> bool {
    name_follows_word(l) && !word_is_expression_tail(l)
}

fn inline_modifier_follows(l: &mut Lx) -> bool {
    if name_follows_word(l) {
        return !word_is_expression_tail(l);
    }
    let c = l.la();
    if (b'0' as i32..=b'9' as i32).contains(&c) || matches!(u8::try_from(c), Ok(b'"' | b'\'' | b'(' | b'{' | b'-')) {
        return true;
    }
    if c != b'\n' as i32 && c != b'\r' as i32 {
        return false;
    }
    while is_space(l.la()) {
        l.advance();
    }
    let w = read_word(l, 12);
    word_in(
        &w,
        &[b"def", b"val", b"var", b"type", b"given", b"class", b"object", b"trait", b"enum", b"final", b"lazy", b"override", b"private", b"protected", b"sealed", b"abstract", b"implicit"],
    )
}

fn is_case_definition_word(l: &mut Lx) -> bool {
    advance_past_blanks(l);
    let w = read_word(l, 7);
    word_in(&w, &[b"class", b"object"])
}

fn is_case_clause_intro(l: &mut Lx) -> bool {
    scan_word(l, b"case") && !is_case_definition_word(l)
}

#[derive(PartialEq, Eq)]
enum CommentAtLayout {
    None,
    Lexed,
    Abort,
    SameLineCode,
}

fn check_comment_at_layout(l: &mut Lx, valid: V) -> CommentAtLayout {
    if l.la() != b'/' as i32 {
        return CommentAtLayout::None;
    }
    l.advance();
    if l.la() == b'*' as i32 && valid & v::BLOCK_COMMENT != 0 {
        l.advance();
        loop {
            consume_block_comment_body(l);
            advance_past_blanks(l);
            if l.eof() || l.la() == b'\n' as i32 || l.la() == b'\r' as i32 {
                l.mark_end();
                return CommentAtLayout::Lexed;
            }
            if l.la() == b'/' as i32 {
                l.advance();
                if l.la() == b'*' as i32 {
                    l.advance();
                    continue;
                }
                if l.la() == b'/' as i32 {
                    while !l.eof() && l.la() != b'\n' as i32 {
                        l.advance();
                    }
                    l.mark_end();
                    return CommentAtLayout::Lexed;
                }
                return CommentAtLayout::SameLineCode;
            }
            return CommentAtLayout::SameLineCode;
        }
    }
    if l.la() == b'/' as i32 || l.la() == b'*' as i32 {
        return CommentAtLayout::Abort;
    }
    CommentAtLayout::None
}

fn skip_delimited(l: &mut Lx, close: i32, escapes: bool) {
    while !l.eof() && l.la() != close && l.la() != b'\n' as i32 {
        if escapes && l.la() == b'\\' as i32 {
            l.advance();
        }
        l.advance();
    }
    if l.la() == close {
        l.advance();
    }
}

fn skip_char_or_quote_tail(l: &mut Lx) -> i32 {
    if l.la() == b'\\' as i32 {
        l.advance();
        if !l.eof() && l.la() != b'\n' as i32 {
            l.advance();
        }
        if l.la() == b'\'' as i32 {
            l.advance();
        }
        return 0;
    }
    if l.eof() || l.la() == b'\n' as i32 {
        return 0;
    }
    let quoted = l.la();
    l.advance();
    if l.la() == b'\'' as i32 {
        l.advance();
        return 0;
    }
    quoted
}

fn rest_of_line_is_blank_or_comments(l: &mut Lx) -> bool {
    loop {
        advance_past_blanks(l);
        if l.eof() || l.la() == b'\n' as i32 || l.la() == b'\r' as i32 {
            return true;
        }
        if l.la() != b'/' as i32 {
            return false;
        }
        l.advance();
        if l.la() == b'/' as i32 {
            while !l.eof() && l.la() != b'\n' as i32 {
                l.advance();
            }
            return true;
        }
        if l.la() != b'*' as i32 {
            return false;
        }
        l.advance();
        if !consume_block_comment_body_on_line(l) {
            return true;
        }
    }
}

fn has_operand(l: &mut Lx) -> bool {
    loop {
        if l.eof() {
            return false;
        }
        if is_space(l.la()) {
            l.advance();
            continue;
        }
        if l.la() != b'/' as i32 {
            return !is_close_or_separator(l.la());
        }
        l.advance();
        if l.la() == b'/' as i32 {
            while !l.eof() && l.la() != b'\n' as i32 {
                l.advance();
            }
            continue;
        }
        if l.la() != b'*' as i32 {
            return true;
        }
        l.advance();
        consume_block_comment_body(l);
    }
}

fn operand_follows(l: &mut Lx) -> bool {
    advance_past_blanks(l) && has_operand(l) && !word_is_expression_tail(l)
}

fn is_leading_infix_continuation(l: &mut Lx) -> bool {
    if is_op_char(l.la()) {
        l.advance();
        while is_op_char(l.la()) {
            l.advance();
        }
        return operand_follows(l);
    }
    if l.la() == b'`' as i32 {
        l.advance();
        while l.la() != b'`' as i32 && !l.eof() {
            l.advance();
        }
        if l.la() != b'`' as i32 {
            return false;
        }
        l.advance();
        return operand_follows(l);
    }
    false
}

#[derive(Default)]
struct LineScan {
    ends_conditional: bool,
    has_case_arrow: bool,
    closes_bracket: bool,
}

fn scan_rest_of_line(l: &mut Lx) -> LineScan {
    let mut depth = 0i32;
    let mut r = LineScan::default();
    while !l.eof() && l.la() != b'\n' as i32 && l.la() != b'\r' as i32 {
        let c = l.la();
        if c == b' ' as i32 || c == b'\t' as i32 {
            l.advance();
        } else if matches!(u8::try_from(c), Ok(b'(' | b'[' | b'{')) {
            depth += 1;
            r.ends_conditional = false;
            l.advance();
        } else if matches!(u8::try_from(c), Ok(b')' | b']' | b'}')) {
            depth -= 1;
            if depth < 0 {
                r.closes_bracket = true;
            }
            r.ends_conditional = false;
            l.advance();
        } else if c == b'"' as i32 || c == b'`' as i32 {
            r.ends_conditional = false;
            l.advance();
            skip_delimited(l, c, c == b'"' as i32);
        } else if c == b'\'' as i32 {
            r.ends_conditional = false;
            l.advance();
            let quoted = skip_char_or_quote_tail(l);
            if matches!(u8::try_from(quoted), Ok(b'(' | b'[' | b'{')) {
                depth += 1;
            } else if matches!(u8::try_from(quoted), Ok(b')' | b']' | b'}')) {
                depth -= 1;
            }
        } else if c == b'/' as i32 {
            l.advance();
            if l.la() == b'/' as i32 {
                break;
            }
            if l.la() == b'*' as i32 {
                l.advance();
                if !consume_block_comment_body_on_line(l) {
                    break;
                }
            } else {
                r.ends_conditional = false;
            }
        } else if c == 0x21D2 {
            if depth == 0 {
                r.has_case_arrow = true;
            }
            r.ends_conditional = false;
            l.advance();
        } else if is_op_char(c) {
            let starts_eq = c == b'=' as i32;
            l.advance();
            let second_gt = l.la() == b'>' as i32;
            let mut extra = 0;
            while is_op_char(l.la()) {
                l.advance();
                extra += 1;
            }
            if depth == 0 && starts_eq && second_gt && extra == 1 {
                r.has_case_arrow = true;
            }
            r.ends_conditional = false;
        } else if is_alpha(c) || c == b'_' as i32 || c == b'$' as i32 {
            let w = read_word(l, 5);
            r.ends_conditional = depth == 0 && word_in(&w, &[b"then", b"do"]);
        } else {
            r.ends_conditional = false;
            l.advance();
        }
    }
    r
}

fn consume_digit_group(l: &mut Lx, saw_sep: &mut bool) -> bool {
    if !(b'0' as i32..=b'9' as i32).contains(&l.la()) {
        return false;
    }
    l.advance();
    loop {
        if (b'0' as i32..=b'9' as i32).contains(&l.la()) {
            l.advance();
        } else if l.la() == b'_' as i32 {
            l.advance();
            if !(b'0' as i32..=b'9' as i32).contains(&l.la()) {
                return false;
            }
            *saw_sep = true;
            l.advance();
        } else {
            return true;
        }
    }
}

fn scan_float_with_separator(l: &mut Lx) -> Option<Ext> {
    let mut int_sep = false;
    if !consume_digit_group(l, &mut int_sep) || !int_sep {
        return None;
    }
    let mut is_float = false;
    let mut ignore = false;
    if l.la() == b'.' as i32 {
        l.advance();
        if !consume_digit_group(l, &mut ignore) {
            return None;
        }
        is_float = true;
    }
    if l.la() == b'e' as i32 || l.la() == b'E' as i32 {
        l.advance();
        if l.la() == b'+' as i32 || l.la() == b'-' as i32 {
            l.advance();
        }
        if !consume_digit_group(l, &mut ignore) {
            return None;
        }
        is_float = true;
    }
    if matches!(u8::try_from(l.la()), Ok(b'd' | b'D' | b'f' | b'F')) {
        l.advance();
        is_float = true;
    }
    if !is_float {
        return None;
    }
    l.mark_end();
    Some(Ext::FloatSep)
}

impl Scanner {
    /// The external token at `pos` where the state takes `valid`: the token and its span, the
    /// scanner's state changed as tree-sitter's (kept by the caller where the token is read).
    pub fn scan(&mut self, src: &[u8], pos: usize, valid: V) -> Option<(Ext, usize, usize)> {
        let mut l = Lx { src, i: pos, start: pos, end: None };
        let r = self.scan_impl(&mut l, valid)?;
        if r == Ext::ColonEol {
            self.after_colon_eol = true;
        } else if r != Ext::BlockComment {
            self.after_colon_eol = false;
        }
        let end = l.end.unwrap_or(l.i);
        Some((r, l.start.min(end), end))
    }

    fn scan_impl(&mut self, l: &mut Lx, valid: V) -> Option<Ext> {
        let prev: i16 = self.indents.last().copied().unwrap_or(-1);
        let prev_width = indent_width(prev);
        let mut newline_count: i16 = 0;
        let mut indentation_size: i16 = 0;

        while is_space(l.la()) {
            if l.la() == b'\n' as i32 {
                newline_count += 1;
                indentation_size = 0;
            } else {
                indentation_size += 1;
            }
            l.skip();
        }

        if valid & v::COMMA_OUTDENT != 0 && l.la() == b',' as i32 && prev != -1 {
            l.mark_end();
            l.advance();
            let mut ends_line = false;
            loop {
                advance_past_blanks(l);
                if l.eof() || l.la() == b'\n' as i32 || l.la() == b'\r' as i32 {
                    ends_line = true;
                    break;
                }
                if l.la() != b'/' as i32 {
                    break;
                }
                l.advance();
                if l.la() == b'/' as i32 {
                    ends_line = true;
                    break;
                }
                if l.la() != b'*' as i32 {
                    break;
                }
                l.advance();
                consume_block_comment_body(l);
            }
            if !ends_line && !scan_rest_of_line(l).closes_bracket {
                return None;
            }
            self.indents.pop();
            return Some(Ext::CommaOutdent);
        }

        if valid & v::OUTDENT != 0
            && self.last_indentation_size != -1
            && at_case_region_width(prev, self.last_indentation_size)
            && (if l.eof() { self.last_column == -1 } else { l.column() == self.last_column })
        {
            l.mark_end();
            if is_case_clause_intro(l) {
                return None;
            }
            self.indents.pop();
            return Some(Ext::Outdent);
        }

        if valid & v::OUTDENT != 0
            && (l.la() == 0
                || (prev != -1 && matches!(u8::try_from(l.la()), Ok(b')' | b']' | b'}')))
                || (self.last_indentation_size != -1 && prev != -1 && self.last_indentation_size < prev_width))
        {
            self.indents.pop();
            return Some(Ext::Outdent);
        }
        self.last_indentation_size = -1;

        let indent_geometry = indentation_size > prev_width || (self.after_colon_eol && indentation_size == prev_width);
        if valid & v::INDENT != 0
            && newline_count > 0
            && !matches!(u8::try_from(l.la()), Ok(b'}' | b')' | b']'))
            && (indent_geometry || (indentation_size == prev_width && l.la() == b'/' as i32))
        {
            l.mark_end();
            match check_comment_at_layout(l, valid) {
                CommentAtLayout::Lexed => return Some(Ext::BlockComment),
                CommentAtLayout::Abort => return None,
                CommentAtLayout::SameLineCode => {
                    let effective = l.column();
                    if effective > prev_width {
                        indentation_size = effective;
                    } else {
                        l.mark_end();
                        return Some(Ext::BlockComment);
                    }
                }
                CommentAtLayout::None => {
                    if !indent_geometry {
                        return None;
                    }
                }
            }
            let mut entry = indentation_size;
            match u8::try_from(l.la()) {
                Ok(b'e' | b'c' | b'f' | b'y' | b'd') => {
                    let w = read_word(l, 8);
                    if word_in(&w, &[b"else", b"catch", b"finally", b"yield", b"do"]) {
                        if valid & v::GATE != 0 && word_in(&w, &[b"else", b"catch", b"finally", b"yield"]) {
                            return Some(Ext::Gate);
                        }
                        return None;
                    }
                    if self.indents.is_empty() && w.as_deref() == Some(b"case") && !is_case_definition_word(l) && scan_rest_of_line(l).has_case_arrow {
                        entry |= CASE_INDENT_FLAG;
                    }
                }
                Ok(b'|' | b'&')
                    if !self.after_colon_eol && is_leading_infix_continuation(l) => {
                        return None;
                    }
                _ => {}
            }
            self.indents.push(entry);
            return Some(Ext::Indent);
        }

        let case_region_close = newline_count > 0 && at_case_region_width(prev, indentation_size);
        if valid & v::OUTDENT != 0 && (l.la() == 0 || (newline_count > 0 && prev != -1 && indentation_size < prev_width) || case_region_close) {
            l.mark_end();
            match check_comment_at_layout(l, valid) {
                CommentAtLayout::Lexed => return Some(Ext::BlockComment),
                CommentAtLayout::Abort => return None,
                CommentAtLayout::SameLineCode => {
                    let effective = l.column();
                    if effective < prev_width {
                        indentation_size = effective;
                    } else {
                        self.last_newline_count = newline_count;
                        self.last_column = effective;
                        self.last_char = (l.la() & 0x7FFF) as i16;
                        l.mark_end();
                        return Some(Ext::BlockComment);
                    }
                }
                CommentAtLayout::None => {}
            }
            self.last_indentation_size = indentation_size;
            self.last_newline_count = newline_count;
            if l.eof() {
                self.last_column = -1;
                self.last_char = 0;
            } else {
                self.last_column = l.column();
                self.last_char = (l.la() & 0x7FFF) as i16;
            }
            if l.la() != 0 {
                let mut probe = Lx { src: l.src, i: l.i, start: l.start, end: l.end };
                if is_leading_infix_continuation(&mut probe) && !scan_rest_of_line(&mut probe).ends_conditional {
                    return None;
                }
                // The C reads on from where the probe stopped.
                l.i = probe.i;
            }
            if case_region_close && is_case_clause_intro(l) {
                return None;
            }
            self.indents.pop();
            return Some(Ext::Outdent);
        }

        if self.last_newline_count > 0 {
            let is_eof = l.eof();
            if (is_eof && self.last_column == -1)
                || (!is_eof && newline_count == 0 && (l.la() & 0x7FFF) as i16 == self.last_char && l.column() == self.last_column)
            {
                newline_count += self.last_newline_count;
            }
        }
        self.last_newline_count = 0;

        if valid & v::END_KW != 0 && l.la() == b'e' as i32 {
            if scan_word(l, b"end") && name_follows_word(l) {
                l.mark_end();
                return Some(Ext::EndKw);
            }
            return None;
        }

        if valid & v::SEMI != 0 && newline_count > 0 {
            l.mark_end();
            let c = l.la();
            if c == b'.' as i32 {
                return None;
            }
            if c == b')' as i32 || c == b']' as i32 || c == b',' as i32 {
                return None;
            }
            if c == b'}' as i32 {
                l.advance();
                loop {
                    advance_past_blanks(l);
                    if matches!(u8::try_from(l.la()), Ok(b'}' | b')' | b']')) {
                        l.advance();
                        continue;
                    }
                    if l.la() == b'/' as i32 {
                        l.advance();
                        if l.la() == b'/' as i32 {
                            return None;
                        }
                        if l.la() == b'*' as i32 {
                            l.advance();
                            consume_block_comment_body(l);
                            continue;
                        }
                    }
                    break;
                }
                if l.la() == b'\n' as i32 || l.la() == b'\r' as i32 || l.eof() {
                    return None;
                }
                return Some(Ext::Semi);
            }
            if c == b'/' as i32 {
                l.advance();
                if l.la() == b'/' as i32 {
                    return None;
                }
                if l.la() == b'*' as i32 && valid & v::BLOCK_COMMENT != 0 {
                    l.advance();
                    consume_block_comment_body(l);
                    advance_past_blanks(l);
                    if !(l.la() == b'\n' as i32 || l.la() == b'\r' as i32 || l.eof()) {
                        self.last_newline_count = newline_count;
                        self.last_column = l.column();
                        self.last_char = (l.la() & 0x7FFF) as i16;
                    }
                    l.mark_end();
                    return Some(Ext::BlockComment);
                }
            }
            if is_op_char(l.la()) || l.la() == b'`' as i32 {
                if newline_count == 1 && is_leading_infix_continuation(l) {
                    return None;
                }
                return Some(Ext::Semi);
            }
            match u8::try_from(l.la()) {
                Ok(b'e')
                    if valid & (v::ELSE | v::EXTENDS | v::GATE) != 0 => {
                        l.advance();
                        if valid & (v::ELSE | v::GATE) != 0 && scan_word(l, b"lse") {
                            if valid & v::GATE != 0 {
                                return Some(Ext::Gate);
                            }
                            return None;
                        }
                        if valid & v::EXTENDS != 0 && scan_word(l, b"xtends") {
                            return None;
                        }
                    }
                Ok(b'c') => {
                    let w = read_word(l, 6);
                    if let Some(w) = w.filter(|w| !w.is_empty()) {
                        if valid & (v::CATCH | v::GATE) != 0 && w == b"catch" {
                            if valid & v::GATE != 0 {
                                return Some(Ext::Gate);
                            }
                            return None;
                        }
                        if w == b"case" && !is_case_definition_word(l) {
                            let line = scan_rest_of_line(l);
                            if line.has_case_arrow && !line.closes_bracket {
                                return None;
                            }
                        }
                    }
                }
                Ok(b'f')
                    if valid & (v::FINALLY | v::GATE) != 0 && scan_word(l, b"finally") => {
                        if valid & v::GATE != 0 {
                            return Some(Ext::Gate);
                        }
                        return None;
                    }
                Ok(b'w')
                    if valid & v::WITH != 0 && scan_word(l, b"with") => {
                        return None;
                    }
                Ok(b'd')
                    if valid & v::DERIVES != 0 && scan_word(l, b"derives") => {
                        return None;
                    }
                Ok(b'u')
                    if valid & v::USES != 0 && scan_word(l, b"uses") => {
                        return None;
                    }
                Ok(b'm')
                    if scan_word(l, b"match") => {
                        return None;
                    }
                _ => {}
            }
            return Some(Ext::Semi);
        }

        let c = l.la();
        let outdent_arm = valid & v::OUTDENT != 0
            && valid & v::GATE == 0
            && newline_count == 0
            && prev != -1
            && ((c == b'e' as i32 && valid & v::ELSE == 0) || (c == b'c' as i32 && valid & v::CATCH == 0) || (c == b'f' as i32 && valid & v::FINALLY == 0));
        const SOFT: [(u8, &[u8], V); 10] = [
            (b'e', b"erased", v::ERASED),
            (b'o', b"open", v::OPEN),
            (b'o', b"opaque", v::OPAQUE),
            (b'i', b"infix", v::INFIX),
            (b't', b"tracked", v::TRACKED),
            (b't', b"transparent", v::TRANSPARENT),
            (b'i', b"inline", v::INLINE),
            (b'i', b"into", v::INTO),
            (b'u', b"update", v::UPDATE),
            (b'c', b"consume", v::CONSUME),
        ];
        let modifier_arm = SOFT.iter().any(|&(first, _, bit)| c == i32::from(first) && valid & bit != 0);
        if outdent_arm || modifier_arm {
            if outdent_arm {
                l.mark_end();
            }
            let w = read_word(l, 12);
            if let Some(w) = w.filter(|w| !w.is_empty()) {
                let modifier = SOFT.iter().find(|&&(_, word, bit)| valid & bit != 0 && w == word).map(|&(_, _, bit)| bit);
                if let Some(m) = modifier {
                    l.mark_end();
                    let follows = if m == v::INLINE { inline_modifier_follows(l) } else { modifier_name_follows(l) };
                    if follows {
                        return Some(Ext::Modifier(m));
                    }
                    return None;
                }
                if outdent_arm && matches!(w.as_slice(), b"else" | b"catch" | b"finally") {
                    self.indents.pop();
                    return Some(Ext::Outdent);
                }
            }
            return None;
        }

        while is_space(l.la()) {
            if l.la() == b'\n' as i32 {
                newline_count += 1;
            }
            l.skip();
        }

        if valid & v::XML != 0 && l.la() == b'<' as i32 {
            l.advance();
            if is_xml_name_start(l.la()) {
                l.mark_end();
                return Some(Ext::Xml);
            }
            return None;
        }

        if valid & v::FLOAT_SEP != 0 && (b'0' as i32..=b'9' as i32).contains(&l.la()) {
            return scan_float_with_separator(l);
        }

        let c = l.la();
        if is_op_char(c) && ((valid & v::COLON_EOL != 0 && c == b':' as i32) || valid & (v::POSTFIX_OP | v::POSTFIX_STAR) != 0 || valid & op_left_class(c) != 0 || valid & v::OP_NAME != 0) {
            let mut op = Vec::with_capacity(3);
            let mut op_len = 0;
            loop {
                if op_len < 3 {
                    op.push(l.la() as u8);
                }
                op_len += 1;
                l.advance();
                if !is_op_char(l.la()) {
                    break;
                }
            }
            l.mark_end();
            if op_len == 1 && op[0] == b':' && l.la() != b'/' as i32 {
                if valid & v::COLON_EOL != 0 && rest_of_line_is_blank_or_comments(l) {
                    return Some(Ext::ColonEol);
                }
                return None;
            }
            let shebang = op_len >= 2 && op[0] == b'#' && op[1] == b'!';
            let op_class = op_left_class(i32::from(op[0]));
            if l.la() == b'/' as i32 && !shebang && valid & (op_class | v::OP_NAME) != 0 {
                let mut ends_in_slash = false;
                let mut at_comment = false;
                while !at_comment {
                    let c = l.la();
                    if c == b'/' as i32 {
                        l.advance();
                        at_comment = l.la() == b'/' as i32 || l.la() == b'*' as i32;
                    } else if is_op_char(c) {
                        l.advance();
                    } else {
                        break;
                    }
                    ends_in_slash = !at_comment && c == b'/' as i32;
                }
                if ends_in_slash && l.la() < 0x80 {
                    l.mark_end();
                    return Some(Ext::Op(if valid & op_class != 0 { op_class } else { v::OP_NAME }));
                }
                return None;
            }
            let postfix = if op_len == 1 && op[0] == b'*' { v::POSTFIX_STAR } else { v::POSTFIX_OP };
            let postfix_ext = if postfix == v::POSTFIX_STAR { Ext::PostfixStar } else { Ext::PostfixOp };
            if valid & postfix == 0 {
                return None;
            }
            if matches!(op[0], b'=' | b'<' | b'>' | b'#' | b'@' | b'?') && op_len <= 3 && matches!(op.as_slice(), b"=" | b"#" | b"@" | b"=>" | b"<-" | b"<:" | b">:" | b"<%" | b"?=>" | b"=>>") {
                return None;
            }
            if !rest_of_line_is_blank_or_comments(l) {
                if is_close_or_separator(l.la()) {
                    return Some(postfix_ext);
                }
                return None;
            }
            if !has_operand(l) {
                return Some(postfix_ext);
            }
            if l.la() == b'.' as i32 || l.la() == b'=' as i32 {
                return None;
            }
            if l.la() == b'@' as i32 {
                return Some(postfix_ext);
            }
            if (b'a' as i32..=b'z' as i32).contains(&l.la()) {
                let w = read_word(l, 10);
                if word_in(
                    &w,
                    &[
                        b"abstract", b"class", b"def", b"enum", b"export", b"final", b"given", b"import", b"implicit", b"lazy", b"object", b"override", b"package", b"private", b"protected", b"sealed",
                        b"trait", b"type", b"val", b"var",
                    ],
                ) {
                    return Some(postfix_ext);
                }
            }
            return None;
        }

        if valid & v::BLOCK_COMMENT != 0 && l.la() == b'/' as i32 && valid & (v::SUPPRESS_BLOCK_COMMENT | v::STRING) == 0 {
            l.advance();
            if l.la() == b'*' as i32 {
                l.advance();
                consume_block_comment_body(l);
                l.mark_end();
                return Some(Ext::BlockComment);
            }
            return None;
        }

        if valid & v::INDENT != 0 && newline_count > 0 && l.la() == b'c' as i32 && prev != -1 && indentation_size == prev_width {
            l.mark_end();
            if is_case_clause_intro(l) {
                self.indents.push(indentation_size | CASE_INDENT_FLAG);
                return Some(Ext::Indent);
            }
            // The probe stopped after `case`'s `ca`; the C then reads `tch`.
            if valid & v::GATE != 0 && l.src[..l.i].ends_with(b"ca") && scan_word(l, b"tch") {
                return Some(Ext::Gate);
            }
            return None;
        }

        if valid & v::GATE != 0 && matches!(u8::try_from(l.la()), Ok(b'c' | b'e' | b'f' | b't' | b'y')) {
            l.mark_end();
            let w = read_word(l, 8);
            if word_in(&w, &[b"catch", b"else", b"finally", b"then", b"yield"]) {
                return Some(Ext::Gate);
            }
            return None;
        }

        None
    }
}
