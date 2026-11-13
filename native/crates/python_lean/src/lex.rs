//! The lexer: tree-sitter-python's lexer and external scanner in one pass. Tokens in order, the
//! layout tokens among them where the scanner gives them, the extras apart (`Lexed`).
//!
//! **Layout.** The scanner runs where the last token or extra ended (`p`), and looks past blanks,
//! line ends and whole comment lines to the next line's indent `L`; `F` is the indent of the first
//! comment line it passed. A layout token is zero-width at `p`:
//!
//! - INDENT, after a block header's `:` (the only place the grammar takes one), where `L` is past
//!   the current indent;
//! - else NEWLINE, where a statement is open;
//! - else DEDENT, while `L` and `F` are both below the current indent (a comment indented as the
//!   block is still the block's: the dedent waits for it);
//! - else nothing: the next comment is lexed, and the scanner runs again from its end.
//!
//! Inside brackets, and inside a string's interpolation, there are none. An indent the grammar
//! does not take (past the current, not after a header) gives no token, as in tree-sitter.
//!
//! **Joined lines.** tree-sitter's scanner gives a NEWLINE only where the parser takes one: where it
//! does not (after `x =`), the line end is a blank, and the statement goes on in the next line
//! (`lex_joined`: the parser names those line ends; the scanner does not run at them).

use crate::block;

/// Why the parser left a file to tree-sitter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The file is past the tape's offsets.
    TooLarge,
    /// The lexer met what it does not read, at this byte.
    Lex(u32),
    /// The parser met what it does not read, at this byte.
    Parse(u32),
}

/// A token's kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum T {
    Eof,
    Newline,
    Indent,
    Dedent,
    Name,
    Int,
    Float,
    // Keywords.
    False,
    None,
    True,
    And,
    As,
    Assert,
    Break,
    Class,
    Continue,
    Def,
    Del,
    Elif,
    Else,
    Except,
    Finally,
    For,
    From,
    Global,
    If,
    Import,
    In,
    Is,
    Lambda,
    Nonlocal,
    Not,
    Or,
    Pass,
    Raise,
    Return,
    Try,
    While,
    With,
    Yield,
    // Punctuation.
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Comma,
    Colon,
    Semi,
    Dot,
    Ellipsis,
    Eq,
    Walrus,
    Arrow,
    At,
    // Operators.
    Plus,
    Minus,
    Star,
    Pow,
    Slash,
    DSlash,
    Percent,
    Amp,
    Pipe,
    Caret,
    Tilde,
    LShift,
    RShift,
    Lt,
    Gt,
    Le,
    Ge,
    EqEq,
    Ne,
    LtGt,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    AtEq,
    DSlashEq,
    PercentEq,
    PowEq,
    RShiftEq,
    LShiftEq,
    AmpEq,
    CaretEq,
    PipeEq,
    // A string's pieces.
    StrStart,
    StrEnd,
    /// Text: hidden in the tape.
    Text,
    Escape,
    EscInterp,
    Backslash,
    /// An interpolation's `{` and `}`.
    IOpen,
    IClose,
    /// `!r`.
    Conversion,
    /// A format specifier's `:`.
    Spec,
}

#[derive(Clone, Copy, Debug)]
pub struct Tok {
    pub kind: T,
    /// A name's soft keyword (`soft::…`; 0: none): in the struct's padding.
    pub soft: u8,
    pub start: u32,
    pub end: u32,
}

/// The soft keywords: names the parser reads as keywords where the grammar takes them.
pub mod soft {
    pub const ASYNC: u8 = 1;
    pub const AWAIT: u8 = 2;
    pub const MATCH: u8 = 3;
    pub const CASE: u8 = 4;
    pub const TYPE: u8 = 5;
    pub const PRINT: u8 = 6;
    pub const EXEC: u8 = 7;
    /// `_`, the wildcard pattern.
    pub const WILD: u8 = 8;
}

/// An extra: a comment or a line continuation.
#[derive(Clone, Copy, Debug)]
pub struct Extra {
    pub start: u32,
    pub end: u32,
    pub comment: bool,
}

pub struct Lexed {
    pub toks: Vec<Tok>,
    pub extras: Vec<Extra>,
}

impl Lexed {
    pub fn text(&self, src: &[u8]) -> String {
        let mut out = String::new();
        let mut e = 0;
        for t in &self.toks {
            while e < self.extras.len() && self.extras[e].start < t.start.max(1) {
                let x = self.extras[e];
                out.push_str(&format!("  extra {}..{} {:?}\n", x.start, x.end, String::from_utf8_lossy(&src[x.start as usize..x.end as usize])));
                e += 1;
            }
            out.push_str(&format!("{:?} {}..{} {:?}\n", t.kind, t.start, t.end, String::from_utf8_lossy(&src[t.start as usize..t.end as usize])));
        }
        out
    }
}

/// A string's delimiter and how its text is read.
#[derive(Clone, Copy)]
struct Quote {
    quote: u8,
    triple: bool,
    raw: bool,
    bytes: bool,
    format: bool,
}

struct Lexer<'s> {
    src: &'s [u8],
    /// The text's byte classes, 64 bytes a step (stage 1).
    b: block::Blocks<'s>,
    toks: Vec<Tok>,
    extras: Vec<Extra>,
    /// Where the last token or extra ended: where the scanner runs.
    p: usize,
    /// Interpolations nested.
    depth: u32,
    /// Where the scanner does not run (each a NEWLINE the parser did not take), in order.
    joins: &'s [u32],
}

const MAX_NESTING: u32 = 64;

pub fn lex(src: &[u8]) -> Result<Lexed, Refusal> {
    lex_joined(src, &[])
}

/// `src` lexed, the line ends at `joins` (where a NEWLINE was, in order) read as blanks.
pub fn lex_joined(src: &[u8], joins: &[u32]) -> Result<Lexed, Refusal> {
    let mut l = Lexer { src, b: block::Blocks::of(src), toks: Vec::with_capacity(src.len() / 3 + 16), extras: Vec::new(), p: 0, depth: 0, joins };
    l.module()?;
    Ok(Lexed { toks: l.toks, extras: l.extras })
}

/// The keywords and soft keywords: each word, its token kind (a soft keyword is a name) and its
/// soft keyword's code.
const WORD_LIST: [(&[u8], T, u8); 41] = [
    (b"False", T::False, 0),
    (b"None", T::None, 0),
    (b"True", T::True, 0),
    (b"and", T::And, 0),
    (b"as", T::As, 0),
    (b"assert", T::Assert, 0),
    (b"break", T::Break, 0),
    (b"class", T::Class, 0),
    (b"continue", T::Continue, 0),
    (b"def", T::Def, 0),
    (b"del", T::Del, 0),
    (b"elif", T::Elif, 0),
    (b"else", T::Else, 0),
    (b"except", T::Except, 0),
    (b"finally", T::Finally, 0),
    (b"for", T::For, 0),
    (b"from", T::From, 0),
    (b"global", T::Global, 0),
    (b"if", T::If, 0),
    (b"import", T::Import, 0),
    (b"in", T::In, 0),
    (b"is", T::Is, 0),
    (b"lambda", T::Lambda, 0),
    (b"nonlocal", T::Nonlocal, 0),
    (b"not", T::Not, 0),
    (b"or", T::Or, 0),
    (b"pass", T::Pass, 0),
    (b"raise", T::Raise, 0),
    (b"return", T::Return, 0),
    (b"try", T::Try, 0),
    (b"while", T::While, 0),
    (b"with", T::With, 0),
    (b"yield", T::Yield, 0),
    (b"async", T::Name, soft::ASYNC),
    (b"await", T::Name, soft::AWAIT),
    (b"match", T::Name, soft::MATCH),
    (b"case", T::Name, soft::CASE),
    (b"type", T::Name, soft::TYPE),
    (b"print", T::Name, soft::PRINT),
    (b"exec", T::Name, soft::EXEC),
    (b"_", T::Name, soft::WILD),
];

/// A word of at most 8 bytes as one integer, little-endian, zeros past its end.
const fn word_key(w: &[u8]) -> u64 {
    let mut k = 0u64;
    let mut i = 0;
    while i < w.len() {
        k |= (w[i] as u64) << (8 * i);
        i += 1;
    }
    k
}

/// A word's slot in `WORDS`, by its key and length: a multiplier under which the 41 words take 41
/// slots of 128 (a perfect hash; `WORDS` is refused at compile time where two share one).
#[inline(always)]
const fn word_slot(key: u64, len: usize) -> usize {
    ((key ^ len as u64).wrapping_mul(0x8f58_640b_360e_7c81) >> 57) as usize
}

/// Each slot's word: its key, its length (0: none), its kind and soft keyword.
static WORDS: [(u64, u8, T, u8); 128] = {
    let mut t = [(0u64, 0u8, T::Name, 0u8); 128];
    let mut i = 0;
    while i < WORD_LIST.len() {
        let (w, kind, soft) = WORD_LIST[i];
        assert!(w.len() <= 8);
        let k = word_key(w);
        let slot = word_slot(k, w.len());
        assert!(t[slot].1 == 0, "two words in a slot");
        t[slot] = (k, w.len() as u8, kind, soft);
        i += 1;
    }
    t
};

/// The word `src[i..end]`'s kind and soft keyword: its first 8 bytes loaded as one integer, masked
/// to its length, hashed to its one slot and compared once (simdjson's check of `true`, `false`
/// and `null`, as one masked compare).
#[inline(always)]
fn keyword(src: &[u8], i: usize, end: usize) -> (T, u8) {
    let len = end - i;
    if len.wrapping_sub(1) >= 8 {
        return (T::Name, 0);
    }
    let raw = match src.get(i..i + 8) {
        Some(b) => u64::from_le_bytes(b.try_into().expect("8 bytes")),
        None => word_key(&src[i..]),
    };
    let w = raw & (u64::MAX >> (64 - 8 * len));
    let (k, l, kind, soft) = WORDS[word_slot(w, len)];
    if k == w && usize::from(l) == len { (kind, soft) } else { (T::Name, 0) }
}

impl<'s> Lexer<'s> {
    /// The scanner does not run here: a line end the parser joined.
    #[inline(always)]
    fn joined(&mut self) -> bool {
        while let Some((&j, rest)) = self.joins.split_first() {
            if j as usize > self.p {
                break;
            }
            self.joins = rest;
            if j as usize == self.p {
                return true;
            }
        }
        false
    }

    #[inline(always)]
    fn at(&self, i: usize) -> u8 {
        self.src.get(i).copied().unwrap_or(0)
    }

    #[inline(always)]
    fn push(&mut self, kind: T, start: usize, end: usize) {
        self.toks.push(Tok { kind, soft: 0, start: start as u32, end: end as u32 });
        self.p = end;
    }

    #[inline(always)]
    fn refuse(&self, at: usize) -> Refusal {
        Refusal::Lex(at as u32)
    }

    /// The width of a zero-width-space-like character tree-sitter-python takes as a blank (U+FEFF,
    /// U+2060, U+200B), or 0.
    #[inline(always)]
    fn odd_blank(&self, i: usize) -> usize {
        match (self.at(i), self.at(i + 1), self.at(i + 2)) {
            (0xef, 0xbb, 0xbf) | (0xe2, 0x81, 0xa0) | (0xe2, 0x80, 0x8b) => 3,
            _ => 0,
        }
    }

    /// The scanner's look from `p`: whether a line ended, the next line's indent `L` and the
    /// first comment line's `F` (-1: none). `None` where the scanner gives no layout token here (a
    /// comment or other text on this line).
    fn scan(&self, p: usize) -> Result<Option<(u32, i64)>, Refusal> {
        let src = self.src;
        let (mut i, mut eol, mut indent, mut first_comment) = (p, false, 0u32, -1i64);
        loop {
            match self.at(i) {
                b'\n' => {
                    eol = true;
                    indent = 0;
                    i += 1;
                }
                b' ' => {
                    indent += 1;
                    i += 1;
                }
                b'\r' => {
                    if self.at(i + 1) != b'\n' {
                        // A lone `\r`: the scanner and the lexer disagree on lines.
                        return Err(self.refuse(i));
                    }
                    indent = 0;
                    i += 1;
                }
                0x0c => {
                    indent = 0;
                    i += 1;
                }
                b'\t' => {
                    indent += 8;
                    i += 1;
                }
                b'#' => {
                    if !eol {
                        return Ok(None);
                    }
                    if first_comment < 0 {
                        first_comment = i64::from(indent);
                    }
                    i = self.b.eol(i);
                    if i < src.len() {
                        i += 1;
                    }
                    indent = 0;
                }
                b'\\' => {
                    i += 1;
                    if self.at(i) == b'\r' {
                        i += 1;
                    }
                    if self.at(i) == b'\n' || i >= src.len() {
                        i += 1;
                    } else {
                        return Ok(None);
                    }
                }
                _ if i >= src.len() => {
                    return Ok(Some((0, first_comment)));
                }
                _ => break,
            }
        }
        Ok(if eol { Some((indent, first_comment)) } else { None })
    }

    /// The whole file.
    fn module(&mut self) -> Result<(), Refusal> {
        let src = self.src;
        let mut indents: Vec<u32> = vec![0];
        // A statement is open (NEWLINE is valid), and the last token was a header's `:`.
        let (mut open, mut header) = (false, false);
        let mut brackets = 0u32;
        loop {
            // The scanner, where it runs: outside brackets.
            // (Only where a line may end before the next token: else the scanner gives nothing.)
            let mut k = self.p;
            while matches!(self.at(k), b' ' | b'\t') {
                k += 1;
            }
            if brackets == 0 && (k >= src.len() || matches!(src[k], b'\n' | b'\r' | b'#' | b'\\' | 0x0c)) && !self.joined() {
                while let Some((indent, first_comment)) = self.scan(self.p)? {
                    let current = *indents.last().expect("an indent");
                    let p = self.p;
                    if header && indent > current {
                        indents.push(indent);
                        self.push(T::Indent, p, p);
                        open = false;
                        header = false;
                        // The scanner again: nothing (the line's indent is the current one).
                        break;
                    } else if open {
                        self.push(T::Newline, p, p);
                        open = false;
                        header = false;
                    } else if indent < current && first_comment < i64::from(current) {
                        indents.pop();
                        self.push(T::Dedent, p, p);
                    } else {
                        break;
                    }
                }
            }
            // Blanks and line ends, then an extra or a token.
            let mut i = k;
            loop {
                let c = self.at(i);
                if c == b' ' || c == b'\t' || c == 0x0c {
                    i = self.b.blank_end(i);
                } else if c == b'\n' {
                    i += 1;
                } else if c == b'\r' {
                    if self.at(i + 1) != b'\n' {
                        return Err(self.refuse(i));
                    }
                    i += 2;
                } else if c >= 0x80 && self.odd_blank(i) > 0 {
                    i += self.odd_blank(i);
                } else {
                    break;
                }
            }
            if i >= src.len() {
                self.push(T::Eof, src.len(), src.len());
                return Ok(());
            }
            let c = src[i];
            if c == b'#' {
                let end = self.b.eol(i);
                self.extras.push(Extra { start: i as u32, end: end as u32, comment: true });
                self.p = end;
                continue;
            }
            if c == b'\\' {
                let mut j = i + 1;
                if self.at(j) == b'\r' {
                    j += 1;
                }
                if self.at(j) == b'\n' {
                    j += 1;
                } else if j < src.len() {
                    return Err(self.refuse(i));
                }
                // Swallowed where a string starts next (the string scanner's blanks).
                let mut k = j;
                while matches!(self.at(k), b' ' | b'\t' | 0x0c) {
                    k += 1;
                }
                if self.string_prefix(k).is_none() {
                    self.extras.push(Extra { start: i as u32, end: j as u32, comment: false });
                }
                self.p = j;
                continue;
            }
            let kind = self.token(i)?;
            match kind {
                T::LParen | T::LBracket | T::LBrace => brackets += 1,
                T::RParen | T::RBracket | T::RBrace => brackets = brackets.saturating_sub(1),
                _ => {}
            }
            open = true;
            header = kind == T::Colon && brackets == 0;
        }
    }

    /// Whether a string starts at `i`: its prefix's length and how it reads.
    fn string_prefix(&self, i: usize) -> Option<(usize, Quote)> {
        let mut j = i;
        let (mut raw, mut bytes, mut format) = (false, false, false);
        loop {
            match self.at(j) {
                b'r' | b'R' => raw = true,
                b'b' | b'B' => bytes = true,
                b'f' | b'F' | b't' | b'T' => format = true,
                b'u' | b'U' => {}
                // A backtick too: the scanner reads Python 2's repr quotes as a string's.
                b'\'' | b'"' | b'`' => break,
                _ => return None,
            }
            j += 1;
        }
        let quote = self.at(j);
        let triple = self.at(j + 1) == quote && self.at(j + 2) == quote;
        Some((j - i, Quote { quote, triple, raw, bytes, format }))
    }

    /// The token at `i` (not a blank, not an extra), and those of a string it starts.
    fn token(&mut self, i: usize) -> Result<T, Refusal> {
        let src = self.src;
        let c = src[i];
        if block::is_word(c) && !c.is_ascii_digit() || c >= 0x80 {
            if let Some((prefix, q)) = self.string_prefix(i) {
                self.string(i, prefix, q)?;
                return Ok(T::StrEnd);
            }
            let end = self.name_end(i)?;
            let (kind, soft) = keyword(src, i, end);
            self.toks.push(Tok { kind, soft, start: i as u32, end: end as u32 });
            self.p = end;
            return Ok(kind);
        }
        if c.is_ascii_digit() || (c == b'.' && self.at(i + 1).is_ascii_digit()) {
            let (kind, end) = self.number(i);
            self.push(kind, i, end);
            return Ok(kind);
        }
        if c == b'\'' || c == b'"' || c == b'`' {
            let (prefix, q) = self.string_prefix(i).expect("a quote");
            self.string(i, prefix, q)?;
            return Ok(T::StrEnd);
        }
        let (kind, len) = self.operator(i)?;
        self.push(kind, i, i + len);
        Ok(kind)
    }

    /// A name's end: ASCII runs a block a step, a character past ASCII by its XID class.
    fn name_end(&self, i: usize) -> Result<usize, Refusal> {
        let src = self.src;
        let mut j = i;
        let mut first = true;
        loop {
            j = self.b.word_end(j);
            if self.at(j) < 0x80 {
                break;
            }
            if self.odd_blank(j) > 0 {
                break;
            }
            let rest = std::str::from_utf8(&src[j..(j + 4).min(src.len())]).or_else(|e| std::str::from_utf8(&src[j..j + e.valid_up_to()])).map_err(|_| self.refuse(j))?;
            let Some(ch) = rest.chars().next() else { return Err(self.refuse(j)) };
            let ok = if first && j == i { unicode_ident::is_xid_start(ch) } else { unicode_ident::is_xid_continue(ch) };
            if !ok {
                if j == i {
                    return Err(self.refuse(j));
                }
                break;
            }
            first = false;
            j += ch.len_utf8();
        }
        Ok(j)
    }

    /// A number at `i`: tree-sitter-python's `integer` or `float`, the longest.
    fn number(&self, i: usize) -> (T, usize) {
        let at = |k: usize| self.at(k);
        let digits = |mut k: usize, ok: &dyn Fn(u8) -> bool| -> usize {
            // `/_?[0-9]+/+` and `/[0-9]+_?/+` alike: digits with single underscores between.
            while ok(at(k)) || (at(k) == b'_' && ok(at(k + 1))) {
                k += 1;
            }
            k
        };
        let dec = |c: u8| c.is_ascii_digit();
        if at(i) == b'0' && matches!(at(i + 1), b'x' | b'X' | b'o' | b'O' | b'b' | b'B') {
            let ok: &dyn Fn(u8) -> bool = match at(i + 1) {
                b'x' | b'X' => &|c: u8| c.is_ascii_hexdigit(),
                b'o' | b'O' => &|c: u8| (b'0'..=b'7').contains(&c),
                _ => &|c: u8| c == b'0' || c == b'1',
            };
            let start = if at(i + 2) == b'_' { i + 3 } else { i + 2 };
            if ok(at(start)) {
                let mut k = digits(start, ok);
                if matches!(at(k), b'l' | b'L') {
                    k += 1;
                }
                return (T::Int, k);
            }
            return (T::Int, i + 1);
        }
        // Decimal: digits (a trailing `_` allowed), a fraction, an exponent, a suffix.
        let int_digits = |mut k: usize| -> usize {
            while dec(at(k)) {
                k += 1;
                while dec(at(k)) {
                    k += 1;
                }
                if at(k) == b'_' {
                    k += 1;
                }
            }
            k
        };
        let mut k = int_digits(i);
        let mut float = false;
        if at(k) == b'.' && (k > i || dec(at(k + 1))) {
            float = true;
            k = int_digits(k + 1);
        }
        if matches!(at(k), b'e' | b'E') {
            let mut e = k + 1;
            if matches!(at(e), b'+' | b'-') {
                e += 1;
            }
            if dec(at(e)) {
                float = true;
                k = int_digits(e);
            }
        }
        if float {
            if matches!(at(k), b'j' | b'J' | b'l' | b'L') {
                k += 1;
            }
            (T::Float, k)
        } else {
            if matches!(at(k), b'j' | b'J' | b'l' | b'L') {
                k += 1;
            }
            (T::Int, k)
        }
    }

    fn operator(&self, i: usize) -> Result<(T, usize), Refusal> {
        let (a, b, c) = (self.at(i), self.at(i + 1), self.at(i + 2));
        Ok(match (a, b, c) {
            (b'*', b'*', b'=') => (T::PowEq, 3),
            (b'/', b'/', b'=') => (T::DSlashEq, 3),
            (b'>', b'>', b'=') => (T::RShiftEq, 3),
            (b'<', b'<', b'=') => (T::LShiftEq, 3),
            (b'.', b'.', b'.') => (T::Ellipsis, 3),
            (b'*', b'*', _) => (T::Pow, 2),
            (b'/', b'/', _) => (T::DSlash, 2),
            (b'>', b'>', _) => (T::RShift, 2),
            (b'<', b'<', _) => (T::LShift, 2),
            (b'<', b'=', _) => (T::Le, 2),
            (b'>', b'=', _) => (T::Ge, 2),
            (b'=', b'=', _) => (T::EqEq, 2),
            (b'!', b'=', _) => (T::Ne, 2),
            (b'<', b'>', _) => (T::LtGt, 2),
            (b'-', b'>', _) => (T::Arrow, 2),
            (b':', b'=', _) => (T::Walrus, 2),
            (b'+', b'=', _) => (T::PlusEq, 2),
            (b'-', b'=', _) => (T::MinusEq, 2),
            (b'*', b'=', _) => (T::StarEq, 2),
            (b'/', b'=', _) => (T::SlashEq, 2),
            (b'@', b'=', _) => (T::AtEq, 2),
            (b'%', b'=', _) => (T::PercentEq, 2),
            (b'&', b'=', _) => (T::AmpEq, 2),
            (b'^', b'=', _) => (T::CaretEq, 2),
            (b'|', b'=', _) => (T::PipeEq, 2),
            (b'(', ..) => (T::LParen, 1),
            (b')', ..) => (T::RParen, 1),
            (b'[', ..) => (T::LBracket, 1),
            (b']', ..) => (T::RBracket, 1),
            (b'{', ..) => (T::LBrace, 1),
            (b'}', ..) => (T::RBrace, 1),
            (b',', ..) => (T::Comma, 1),
            (b':', ..) => (T::Colon, 1),
            (b';', ..) => (T::Semi, 1),
            (b'.', ..) => (T::Dot, 1),
            (b'=', ..) => (T::Eq, 1),
            (b'@', ..) => (T::At, 1),
            (b'+', ..) => (T::Plus, 1),
            (b'-', ..) => (T::Minus, 1),
            (b'*', ..) => (T::Star, 1),
            (b'/', ..) => (T::Slash, 1),
            (b'%', ..) => (T::Percent, 1),
            (b'&', ..) => (T::Amp, 1),
            (b'|', ..) => (T::Pipe, 1),
            (b'^', ..) => (T::Caret, 1),
            (b'~', ..) => (T::Tilde, 1),
            (b'<', ..) => (T::Lt, 1),
            (b'>', ..) => (T::Gt, 1),
            _ => return Err(self.refuse(i)),
        })
    }

    /// A string from `i`: its start (prefix and quotes), text, escapes, interpolations, end.
    fn string(&mut self, i: usize, prefix: usize, q: Quote) -> Result<(), Refusal> {
        let src = self.src;
        let open = prefix + if q.triple { 3 } else { 1 };
        self.push(T::StrStart, i, i + open);
        let mut j = i + open;
        // The text since the last piece, and whether any of it is more than a raw string's
        // backslashes (which the scanner steps over without counting them as text: where nothing
        // else is, its next token starts at them).
        let mut text = j;
        let mut content = false;
        loop {
            let s = self.b.string_stop(j, q.quote, q.format);
            if s >= src.len() {
                return Err(self.refuse(s));
            }
            content |= s > j;
            let c = src[s];
            if c == q.quote {
                if !q.triple || (self.at(s + 1) == q.quote && self.at(s + 2) == q.quote) {
                    let close = if q.triple { 3 } else { 1 };
                    if content {
                        self.text(text, s);
                        self.push(T::StrEnd, s, s + close);
                    } else {
                        self.push(T::StrEnd, text, s + close);
                    }
                    return Ok(());
                }
                j = s + 1;
                content = true;
            } else if c == b'\n' {
                if !q.triple {
                    return Err(self.refuse(s));
                }
                j = s + 1;
                content = true;
            } else if c == b'\\' {
                let next = self.at(s + 1);
                if q.raw {
                    // A raw string's backslash is text, and so is a quote, a backslash or a line end
                    // after it (the scanner's).
                    j = s + 1;
                    if next == q.quote || next == b'\\' || next == b'\n' {
                        j += 1;
                    } else if next == b'\r' {
                        j += if self.at(s + 2) == b'\n' { 2 } else { 1 };
                    }
                    continue;
                }
                if q.bytes && matches!(next, b'N' | b'u' | b'U') {
                    j = s + 2;
                    content = true;
                    continue;
                }
                self.text(text, s);
                self.backslash(s);
                j = self.p;
                text = j;
                content = false;
            } else {
                // `{` or `}`. A raw string's backslashes alone before it are read as the lexer
                // reads them outside the scanner: escapes, or a lone `\`.
                if content {
                    self.text(text, s);
                } else {
                    let mut k = text;
                    while k < s {
                        self.backslash(k);
                        k = self.p;
                    }
                }
                if c == b'{' && self.at(s + 1) != b'{' {
                    j = self.interpolation(s)?;
                } else {
                    if self.at(s + 1) != c {
                        return Err(self.refuse(s));
                    }
                    self.push(T::EscInterp, s, s + 2);
                    j = s + 2;
                }
                text = j;
                content = false;
            }
        }
    }

    /// The escape sequence, or the lone `\`, at `s`.
    fn backslash(&mut self, s: usize) {
        match self.escape(s) {
            Some(end) => self.push(T::Escape, s, end),
            None => self.push(T::Backslash, s, s + 1),
        }
    }

    #[inline(always)]
    fn text(&mut self, from: usize, to: usize) {
        if to > from {
            self.push(T::Text, from, to);
        }
    }

    /// An escape sequence's end, where one starts at `s` (tree-sitter-python's `escape_sequence`).
    fn escape(&self, s: usize) -> Option<usize> {
        let at = |k: usize| self.at(k);
        let hex = |from: usize, n: usize| (from..from + n).all(|k| at(k).is_ascii_hexdigit());
        let c = at(s + 1);
        Some(match c {
            b'u' if hex(s + 2, 4) => s + 6,
            b'U' if hex(s + 2, 8) => s + 10,
            b'x' if hex(s + 2, 2) => s + 4,
            b'0'..=b'9' => {
                let mut k = s + 2;
                while k < s + 4 && at(k).is_ascii_digit() {
                    k += 1;
                }
                k
            }
            b'\r' => {
                if at(s + 2) == b'\n' {
                    s + 3
                } else {
                    s + 2
                }
            }
            b'\n' | b'\'' | b'"' | b'a' | b'b' | b'f' | b'r' | b'n' | b't' | b'v' | b'\\' => s + 2,
            b'N' if at(s + 2) == b'{' => {
                let mut k = s + 3;
                while k < self.src.len() && at(k) != b'}' {
                    k += 1;
                }
                if k == s + 3 || k >= self.src.len() {
                    return None;
                }
                k + 1
            }
            _ => return None,
        })
    }

    /// An interpolation from its `{` at `s`: its expression's tokens, then `=`, `!x`, a format
    /// specifier (with interpolations of its own), and `}`. Where it ends.
    fn interpolation(&mut self, s: usize) -> Result<usize, Refusal> {
        self.depth += 1;
        if self.depth > MAX_NESTING {
            return Err(self.refuse(s));
        }
        self.push(T::IOpen, s, s + 1);
        let src = self.src;
        let mut brackets = 0u32;
        let mut i = s + 1;
        loop {
            // Blanks and line ends (a triple-quoted string's interpolation may span lines).
            loop {
                match self.at(i) {
                    b' ' | b'\t' | 0x0c | b'\n' => i += 1,
                    b'\r' if self.at(i + 1) == b'\n' => i += 2,
                    _ => break,
                }
            }
            if i >= src.len() {
                return Err(self.refuse(i));
            }
            let c = src[i];
            if brackets == 0 {
                if c == b'}' {
                    self.push(T::IClose, i, i + 1);
                    self.depth -= 1;
                    return Ok(i + 1);
                }
                if c == b'!' && self.at(i + 1) != b'=' {
                    if !self.at(i + 1).is_ascii_lowercase() {
                        return Err(self.refuse(i));
                    }
                    self.push(T::Conversion, i, i + 2);
                    i += 2;
                    continue;
                }
                if c == b':' {
                    self.push(T::Spec, i, i + 1);
                    let end = self.spec(i + 1)?;
                    self.push(T::IClose, end, end + 1);
                    self.depth -= 1;
                    return Ok(end + 1);
                }
            }
            if c == b'#' || c == b'\\' {
                return Err(self.refuse(i));
            }
            let kind = self.token(i)?;
            match kind {
                T::LParen | T::LBracket | T::LBrace => brackets += 1,
                T::RParen | T::RBracket | T::RBrace => {
                    if brackets == 0 {
                        return Err(self.refuse(i));
                    }
                    brackets -= 1;
                }
                _ => {}
            }
            i = self.p;
        }
    }

    /// A format specifier's text from `i` (after its `:`), with its interpolations: where its `}` is.
    fn spec(&mut self, mut i: usize) -> Result<usize, Refusal> {
        let src = self.src;
        loop {
            let s = self.b.spec_stop(i);
            if s >= src.len() || src[s] == b'\n' {
                return Err(self.refuse(s));
            }
            if src[s] == b'}' {
                return Ok(s);
            }
            if self.at(s + 1) == b'{' {
                return Err(self.refuse(s));
            }
            i = self.interpolation(s)?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords_are_found_by_their_one_slot() {
        for &(w, kind, soft) in &WORD_LIST {
            // At the text's end (padded) and inside it (8 bytes loaded past the word).
            for text in [w.to_vec(), [w, b" + xyzzy_long"].concat()] {
                assert_eq!(keyword(&text, 0, w.len()), (kind, soft), "{}", String::from_utf8_lossy(w));
            }
            // Not a word with a byte more or less, or another case.
            let longer = [w, b"x"].concat();
            assert_eq!(keyword(&longer, 0, longer.len()), (T::Name, 0));
            assert_eq!(keyword(w, 0, w.len() - 1).1, WORD_LIST.iter().find(|e| e.0 == &w[..w.len() - 1]).map_or(0, |e| e.2));
            let upper = w.to_ascii_uppercase();
            if upper != w {
                assert_eq!(keyword(&upper, 0, upper.len()), (T::Name, 0));
            }
        }
        assert_eq!(keyword(b"continued", 0, 9), (T::Name, 0));
        assert_eq!(keyword(b"x_continue", 2, 10), (T::Continue, 0));
    }

    fn kinds(src: &str) -> Vec<T> {
        lex(src.as_bytes()).unwrap().toks.iter().map(|t| t.kind).collect()
    }

    #[test]
    fn layout_tokens() {
        use T::*;
        assert_eq!(kinds("if a:\n    b\nc\n"), [If, Name, Colon, Indent, Name, Newline, Dedent, Name, Newline, Eof]);
        assert_eq!(kinds("x = (1,\n 2)\n"), [Name, Eq, LParen, Int, Comma, Int, RParen, Newline, Eof]);
        // A comment at the block's indent keeps the dedent waiting.
        let l = lex(b"if a:\n    b\n    # c\nd\n").unwrap();
        let dedent = l.toks.iter().find(|t| t.kind == Dedent).unwrap();
        assert_eq!(dedent.start, 19);
        let l = lex(b"if a:\n    b\n# c\nd\n").unwrap();
        let dedent = l.toks.iter().find(|t| t.kind == Dedent).unwrap();
        assert_eq!(dedent.start, 11);
    }

    #[test]
    fn strings_and_numbers() {
        use T::*;
        assert_eq!(kinds("f'a{b!r:>{w}}c'\n"), [StrStart, Text, IOpen, Name, Conversion, Spec, IOpen, Name, IClose, IClose, Text, StrEnd, Newline, Eof]);
        assert_eq!(kinds("'\\n\\d'\n"), [StrStart, Escape, Backslash, Text, StrEnd, Newline, Eof]);
        assert_eq!(kinds("1..real\n"), [Float, Dot, Name, Newline, Eof]);
        assert_eq!(kinds("0x_ffL 1_0.0_1e1_0j .5\n"), [Int, Float, Float, Newline, Eof]);
    }
}
