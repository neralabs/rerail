//! The lexer: tree-sitter-go's lexer, read where the parser asks. tree-sitter-go has no external
//! scanner. The lexer keeps no state: the parser asks for the next token at a byte (`Lexer::token`),
//! and reads itself the few places its parse state lexes otherwise (a string's pieces).
//!
//! **Line ends.** tree-sitter-go's terminator is `\n` (or `;`): a `\n` is a token where the parse
//! state takes a terminator, and white space elsewhere. The lexer does not know the state, so each
//! token says where the first `\n` before it is (outside comments, `Tok::nl`), and the parser,
//! where it takes a terminator, reads that `\n` as the token.
//!
//! Every word is a `Name`, with its word code (`w::…`; 0: none) from one hash probe: tree-sitter
//! reads a keyword only where its parse state takes it (keyword extraction), so the parser decides.
//!
//! Comments are extras, in a list of their own, written where the parser next opens a node or
//! reads a token.

use crate::block::{self, Blocks};

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

pub type R<X> = Result<X, Refusal>;

/// A token's kind. Punctuation is lexed longest first, as tree-sitter's lexer does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum T {
    Eof,
    Name,
    Int,
    Float,
    Imaginary,
    Rune,
    /// An interpreted string, `"` to `"` (its pieces are read by the parser from its bytes).
    Quote,
    /// A raw string, `` ` `` to `` ` ``.
    Raw,
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Semi,
    Comma,
    Dot,
    DotDotDot,
    Colon,
    ColonEq,
    Eq,
    EqEq,
    Bang,
    Ne,
    Tilde,
    Plus,
    PlusPlus,
    PlusEq,
    Minus,
    MinusMinus,
    MinusEq,
    Star,
    StarEq,
    Slash,
    SlashEq,
    Percent,
    PercentEq,
    Caret,
    CaretEq,
    Amp,
    AmpAmp,
    AmpEq,
    AmpCaret,
    AmpCaretEq,
    Pipe,
    PipePipe,
    PipeEq,
    Lt,
    Le,
    LtLt,
    LtLtEq,
    LtMinus,
    Gt,
    Ge,
    GtGt,
    GtGtEq,
}

/// No `\n` before a token (`Tok::nl`).
pub const NO_NL: u32 = u32::MAX;

#[derive(Clone, Copy, Debug)]
pub struct Tok {
    pub kind: T,
    /// A name's word code (`w::…`; 0: none).
    pub word: u8,
    pub start: u32,
    pub end: u32,
    /// The first `\n` between the token before and this one, outside comments (`NO_NL`: none).
    pub nl: u32,
    /// The `\n` tree-sitter reads as a terminator here: the last of the first run of white space
    /// that holds one (`NO_NL`: none).
    pub nl_last: u32,
}

/// The token a refusal is read as where the parser comes to it: an `Eof` with word `u8::MAX`, at the
/// refused byte (at 0 for a text too large).
#[cold]
pub fn refused(r: Refusal) -> Tok {
    let at = match r {
        Refusal::Lex(at) | Refusal::Parse(at) => at,
        Refusal::TooLarge => 0,
    };
    Tok { kind: T::Eof, word: u8::MAX, start: at, end: at, nl: NO_NL, nl_last: NO_NL }
}

/// A comment (the only extra).
#[derive(Clone, Copy, Debug)]
pub struct Extra {
    pub start: u32,
    pub end: u32,
}

/// Word codes: the keywords and the words the grammar names.
#[allow(dead_code)]
pub mod w {
    pub const NONE: u8 = 0;
    macro_rules! words {
        ($($name:ident = $text:literal,)*) => {
            words!(@ 1u8; $($name = $text,)*);
            /// Each word's text, by code (0: none).
            pub const TEXT: &[&[u8]] = &[b"", $($text),*];
        };
        (@ $n:expr; $name:ident = $text:literal, $($rest:ident = $rt:literal,)*) => {
            pub const $name: u8 = $n;
            words!(@ $n + 1u8; $($rest = $rt,)*);
        };
        (@ $n:expr;) => {};
    }
    words! {
        BREAK = b"break", CASE = b"case", CHAN = b"chan", CONST = b"const", CONTINUE = b"continue",
        DEFAULT = b"default", DEFER = b"defer", ELSE = b"else", FALLTHROUGH = b"fallthrough", FOR = b"for",
        FUNC = b"func", GO = b"go", GOTO = b"goto", IF = b"if", IMPORT = b"import", INTERFACE = b"interface",
        MAP = b"map", PACKAGE = b"package", RANGE = b"range", RETURN = b"return", SELECT = b"select",
        STRUCT = b"struct", SWITCH = b"switch", TYPE = b"type", VAR = b"var",
        // Words that are tokens of the grammar but names elsewhere.
        NIL = b"nil", TRUE = b"true", FALSE = b"false", IOTA = b"iota", NEW = b"new", MAKE = b"make",
    }
}

rerail_simd8::keyword_switch!(
    /// The keyword switch (`rerail_simd8::words`) over `w::TEXT`: a word's code is its index there.
    static WORDS = w::TEXT, fold: false
);

/// The word `src[i..end]`'s code (0: none), in its case: one probe of the keyword switch.
#[inline(always)]
pub fn word(src: &[u8], i: usize, end: usize) -> u8 {
    WORDS.find(src, i, end) as u8
}

/// The character at `at` (a UTF-8 lead byte at or past 0x80) and its length.
fn decode(src: &[u8], at: usize) -> Option<(char, usize)> {
    let s = std::str::from_utf8(&src[at..(at + 4).min(src.len())]).or_else(|e| std::str::from_utf8(&src[at..at + e.valid_up_to()])).ok()?;
    let c = s.chars().next()?;
    Some((c, c.len_utf8()))
}

/// `FIRST`'s bits for a byte: what may follow a token before the next one starts (a blank, a line
/// end, a comment's `/`)...
const GAP: u8 = 1;
/// ...and what does not start an ASCII name, so that a 0 bit is a name's first byte.
const NAME: u8 = 2;

/// A byte's `GAP` and `NAME` bits, by one load.
static FIRST: [u8; 256] = {
    let mut t = [NAME; 256];
    let mut c = 0;
    while c < 256 {
        let b = c as u8;
        if matches!(b, b' ' | b'\t' | b'\x0b' | b'\x0c' | b'\r' | b'\n' | b'/') {
            t[c] |= GAP;
        }
        if b.is_ascii_alphabetic() || b == b'_' {
            t[c] &= !NAME;
        }
        c += 1;
    }
    t
};

/// The punctuators of one byte that start no longer one, by their byte (`T::Eof`: none).
static ONE: [T; 256] = {
    let mut t = [T::Eof; 256];
    t[b'(' as usize] = T::LParen;
    t[b')' as usize] = T::RParen;
    t[b'[' as usize] = T::LBracket;
    t[b']' as usize] = T::RBracket;
    t[b'{' as usize] = T::LBrace;
    t[b'}' as usize] = T::RBrace;
    t[b';' as usize] = T::Semi;
    t[b',' as usize] = T::Comma;
    t[b'~' as usize] = T::Tilde;
    t
};

pub struct Lexer<'s> {
    pub src: &'s [u8],
    pub blocks: Blocks,
}

impl<'s> Lexer<'s> {
    pub fn new(src: &'s [u8]) -> Self {
        Lexer { src, blocks: Blocks::of(src) }
    }

    #[inline(always)]
    pub fn at(&self, i: usize) -> u8 {
        self.src.get(i).copied().unwrap_or(0)
    }

    /// From `at`: white space and comments passed (the comments pushed to `extras`); where the next
    /// token starts, and the first `\n` passed outside comments (`NO_NL`: none).
    #[inline]
    pub fn skip(&self, mut at: usize, extras: &mut Vec<Extra>) -> R<(usize, u32, u32)> {
        // Most tokens follow the one before at once (`f(`, `a.b`): no blank, line end or comment to
        // pass. Past the text's end the byte read is 0, which starts none.
        if FIRST[usize::from(self.at(at))] & GAP == 0 {
            return Ok((at, NO_NL, NO_NL));
        }
        let (mut nl, mut last, mut closed) = (NO_NL, NO_NL, false);
        loop {
            at = self.blocks.blank_end(at);
            match self.at(at) {
                b'\n' if at < self.src.len() => {
                    if nl == NO_NL {
                        nl = at as u32;
                    }
                    if last == NO_NL || !closed {
                        last = at as u32;
                    }
                    at += 1;
                }
                b'/' => {
                    let Some(x) = self.comment(at)? else { return Ok((at, nl, last)) };
                    extras.push(x);
                    at = x.end as usize;
                    closed = last != NO_NL;
                }
                _ => return Ok((at, nl, last)),
            }
        }
    }

    /// The comment starting at `at` (its `/`), if one does; an unclosed `/*` refused.
    #[inline]
    pub fn comment(&self, at: usize) -> R<Option<Extra>> {
        match self.at(at + 1) {
            b'/' => Ok(Some(Extra { start: at as u32, end: self.blocks.eol(at + 2) as u32 })),
            b'*' => {
                let mut i = at + 2;
                loop {
                    i = self.blocks.star(i);
                    if i >= self.src.len() {
                        return Err(Refusal::Lex(at as u32));
                    }
                    if self.src[i] == b'*' && self.at(i + 1) == b'/' {
                        break Ok(Some(Extra { start: at as u32, end: i as u32 + 2 }));
                    }
                    i += 1;
                }
            }
            _ => Ok(None),
        }
    }

    /// The next token from `at`, white space and comments passed.
    #[inline]
    pub fn token(&self, at: usize, extras: &mut Vec<Extra>) -> R<Tok> {
        let (s, nl, nl_last) = self.skip(at, extras)?;
        let (kind, end, word) = self.token_at(s)?;
        Ok(Tok { kind, word, start: s as u32, end: end as u32, nl, nl_last })
    }

    /// Up to `n` tokens from `at` pushed onto `out`, comments onto `extras`: where the next is
    /// lexed. A run ends early after `Eof`, or after a refusal, written as `refused` writes it.
    ///
    /// simdjson's stage 2 in page 31's form (`rerail_simd8::blocks`): each token's start is the
    /// next one of stage 1's start masks, the gap's first and last line ends scans bounded by it,
    /// an ASCII name's end its word run, a short decimal's its run and one 8-byte test; the token
    /// written into reserved room, the length set once. A token that is not one of those is read
    /// by `token_at`; a comment in the gap sends the token to `token` from the gap's start.
    #[inline(always)]
    pub fn run(&self, mut at: usize, extras: &mut Vec<Extra>, out: &mut Vec<Tok>, n: usize) -> usize {
        let len = self.src.len();
        let mut cur = rerail_simd8::blocks::Cursor::default();
        out.reserve(n);
        let base = out.len();
        let w = out.spare_capacity_mut().as_mut_ptr();
        let mut k = 0;
        while k < n {
            let c0 = self.at(at);
            let (s, nl, nl_last) = if FIRST[usize::from(c0)] & GAP == 0 {
                (at, NO_NL, NO_NL)
            } else if c0 == b'/' {
                (usize::MAX, NO_NL, NO_NL)
            } else if matches!(c0, b' ' | b'\t') && FIRST[usize::from(self.at(at + 1))] & GAP == 0 {
                // One blank, the most common gap: no walk and no line end to look for.
                (at + 1, NO_NL, NO_NL)
            } else {
                // A gap of blanks and line ends: the next start ends it.
                let s = self.blocks.next_start(&mut cur, at);
                match self.blocks.eol_in(at, s) {
                    None => (s, NO_NL, NO_NL),
                    Some(e) => (s, e as u32, self.blocks.last_eol_in(e, s).map_or(e as u32, |l| l as u32)),
                }
            };
            let c = if s < len { self.src[s] } else { 0 };
            let one = ONE[usize::from(c)];
            let t = if s >= len || c == b'/' {
                self.token(at, extras).unwrap_or_else(refused)
            } else if one != T::Eof {
                Tok { kind: one, word: 0, start: s as u32, end: s as u32 + 1, nl, nl_last }
            } else if FIRST[usize::from(c)] & NAME == 0 {
                let end = self.blocks.word_end(s + 1);
                if self.at(end) < 0x80 {
                    Tok { kind: T::Name, word: word(self.src, s, end), start: s as u32, end: end as u32, nl, nl_last }
                } else {
                    self.token(at, extras).unwrap_or_else(refused)
                }
            } else if let Some(end) = self.decimal(s) {
                Tok { kind: T::Int, word: 0, start: s as u32, end: end as u32, nl, nl_last }
            } else {
                match self.token_at(s) {
                    Ok((kind, end, word)) => Tok { kind, word, start: s as u32, end: end as u32, nl, nl_last },
                    Err(r) => refused(r),
                }
            };
            // SAFETY: `k < n`, and `n` tokens' room is reserved past `base`.
            unsafe { (*w.add(k)).write(t) };
            k += 1;
            at = t.end as usize;
            if t.kind == T::Eof {
                break;
            }
        }
        // SAFETY: the first `k` tokens past `base` are written.
        unsafe { out.set_len(base + k) };
        at
    }

    /// A decimal integer of up to 8 digits at `s`, with no `_`, fraction, exponent or `i` after it,
    /// no character past ASCII after it, and no leading `0`: its end, from its word run and one
    /// test of its 8 bytes. `number` reads every other number.
    #[inline(always)]
    fn decimal(&self, s: usize) -> Option<usize> {
        let c = self.src[s];
        if !(b'1'..=b'9').contains(&c) {
            return None;
        }
        let end = self.blocks.word_end(s + 1);
        let n = end - s;
        let d = self.at(end);
        if n > 8 || d == b'.' || d >= 0x80 || !rerail_simd8::swar::digits(rerail_simd8::swar::load8(self.src, s), n) {
            return None;
        }
        Some(end)
    }

    /// The token at `s` (no white space before it): its kind, end and word code. Punctuation is
    /// read here, inlined where the token is taken; the rest, `token_rare`'s, is called.
    #[inline(always)]
    fn token_at(&self, s: usize) -> R<(T, usize, u8)> {
        use T::*;
        if s >= self.src.len() {
            return Ok((Eof, s, 0));
        }
        let c = self.src[s];
        let next = self.at(s + 1);
        let (kind, len) = match c {
            b'.' if next.is_ascii_digit() => return self.token_rare(s),
            b'(' => (LParen, 1),
            b')' => (RParen, 1),
            b'[' => (LBracket, 1),
            b']' => (RBracket, 1),
            b'{' => (LBrace, 1),
            b'}' => (RBrace, 1),
            b';' => (Semi, 1),
            b',' => (Comma, 1),
            b'~' => (Tilde, 1),
            b':' => match next {
                b'=' => (ColonEq, 2),
                _ => (Colon, 1),
            },
            b'.' => {
                if next == b'.' && self.at(s + 2) == b'.' {
                    (DotDotDot, 3)
                } else {
                    (Dot, 1)
                }
            }
            b'=' => match next {
                b'=' => (EqEq, 2),
                _ => (Eq, 1),
            },
            b'!' => match next {
                b'=' => (Ne, 2),
                _ => (Bang, 1),
            },
            b'+' => match next {
                b'+' => (PlusPlus, 2),
                b'=' => (PlusEq, 2),
                _ => (Plus, 1),
            },
            b'-' => match next {
                b'-' => (MinusMinus, 2),
                b'=' => (MinusEq, 2),
                _ => (Minus, 1),
            },
            b'*' => match next {
                b'=' => (StarEq, 2),
                _ => (Star, 1),
            },
            b'/' => match next {
                b'=' => (SlashEq, 2),
                _ => (Slash, 1),
            },
            b'%' => match next {
                b'=' => (PercentEq, 2),
                _ => (Percent, 1),
            },
            b'^' => match next {
                b'=' => (CaretEq, 2),
                _ => (Caret, 1),
            },
            b'&' => match (next, self.at(s + 2)) {
                (b'&', _) => (AmpAmp, 2),
                (b'=', _) => (AmpEq, 2),
                (b'^', b'=') => (AmpCaretEq, 3),
                (b'^', _) => (AmpCaret, 2),
                _ => (Amp, 1),
            },
            b'|' => match next {
                b'|' => (PipePipe, 2),
                b'=' => (PipeEq, 2),
                _ => (Pipe, 1),
            },
            b'<' => match (next, self.at(s + 2)) {
                (b'<', b'=') => (LtLtEq, 3),
                (b'<', _) => (LtLt, 2),
                (b'=', _) => (Le, 2),
                (b'-', _) => (LtMinus, 2),
                _ => (Lt, 1),
            },
            b'>' => match (next, self.at(s + 2)) {
                (b'>', b'=') => (GtGtEq, 3),
                (b'>', _) => (GtGt, 2),
                (b'=', _) => (Ge, 2),
                _ => (Gt, 1),
            },
            _ => return self.token_rare(s),
        };
        Ok((kind, s + len, 0))
    }

    /// `token_at`'s tokens that are no punctuator: names, numbers, literals (and a refusal),
    /// outlined so that punctuation is read with no frame to save.
    #[inline(never)]
    fn token_rare(&self, s: usize) -> R<(T, usize, u8)> {
        use T::*;
        let c = self.src[s];
        let refuse = Err(Refusal::Lex(s as u32));
        // An ASCII name: its end is where its word bytes stop, unless a character past ASCII goes
        // on there.
        if FIRST[usize::from(c)] & NAME == 0 {
            let end = self.blocks.word_end(s + 1);
            let end = if self.at(end) < 0x80 { end } else { self.name_end(s)? };
            return Ok((Name, end, word(self.src, s, end)));
        }
        match c {
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                let end = self.name_end(s)?;
                return Ok((Name, end, word(self.src, s, end)));
            }
            b'0'..=b'9' => return self.number(s),
            b'.' => self.number(s),
            b'"' => {
                return match self.string_end(s) {
                    Some(end) => Ok((Quote, end, 0)),
                    None => refuse,
                };
            }
            b'`' => {
                let end = self.blocks.tick(s + 1);
                if end >= self.src.len() {
                    return refuse;
                }
                return Ok((Raw, end + 1, 0));
            }
            b'\'' => match self.rune_end(s) {
                Some(end) => return Ok((Rune, end, 0)),
                None => return refuse,
            },
            c if c >= 0x80 => {
                let end = self.name_end(s)?;
                if end == s {
                    return refuse;
                }
                Ok((Name, end, 0))
            }
            _ => refuse,
        }
    }

    /// The end of a name from `s` (`[_\p{XID_Start}][_\p{XID_Continue}]*`): ASCII word bytes a block
    /// a step, a character past ASCII by its class. `s` itself where no name starts there.
    pub fn name_end(&self, s: usize) -> R<usize> {
        let c = self.at(s);
        if c.is_ascii_digit() || (c < 0x80 && !block::is_word(c)) {
            return Ok(s);
        }
        let mut i = s;
        loop {
            i = self.blocks.word_end(i);
            if self.at(i) < 0x80 || i >= self.src.len() {
                return Ok(i);
            }
            let Some((ch, n)) = decode(self.src, i) else { return Err(Refusal::Lex(i as u32)) };
            let ok = if i == s { unicode_ident::is_xid_start(ch) } else { unicode_ident::is_xid_continue(ch) };
            if !ok {
                return Ok(i);
            }
            i += n;
        }
    }

    /// `digit (_? digit)*` from `i`: its end, `None` where no digit starts it.
    fn digits(&self, mut i: usize, digit: impl Fn(u8) -> bool) -> Option<usize> {
        if !digit(self.at(i)) {
            return None;
        }
        loop {
            i += 1;
            while digit(self.at(i)) {
                i += 1;
            }
            if self.at(i) == b'_' && digit(self.at(i + 1)) {
                i += 1;
                continue;
            }
            return Some(i);
        }
    }

    /// `_? digit (_? digit)*` from `i` (a prefix's digits): its end.
    fn prefixed_digits(&self, i: usize, digit: impl Fn(u8) -> bool + Copy) -> Option<usize> {
        if self.at(i) == b'_' { self.digits(i + 1, digit) } else { self.digits(i, digit) }
    }

    /// A decimal exponent from `i` (`[eE][+-]?D`, or `[pP][+-]?D` where `hex`): its end.
    fn exponent(&self, i: usize, hex: bool) -> Option<usize> {
        let c = self.at(i);
        if !(if hex { matches!(c, b'p' | b'P') } else { matches!(c, b'e' | b'E') }) {
            return None;
        }
        let k = if matches!(self.at(i + 1), b'+' | b'-') { i + 2 } else { i + 1 };
        self.digits(k, |c| c.is_ascii_digit())
    }

    /// A number at `s` (a digit, or a `.` before one): the longest of `int_literal`,
    /// `float_literal` and `imaginary_literal` (one of them and `i`).
    fn number(&self, s: usize) -> R<(T, usize, u8)> {
        let dec = |c: u8| c.is_ascii_digit();
        let hexd = |c: u8| c.is_ascii_hexdigit();
        let mut best = (T::Int, s);
        let mut take = |kind: T, end: usize| {
            if end > best.1 {
                best = (kind, end);
            }
        };
        let (c0, c1) = (self.at(s), self.at(s + 1));
        if c0 == b'0' && matches!(c1, b'x' | b'X') {
            let int = self.prefixed_digits(s + 2, hexd);
            if let Some(e) = int {
                take(T::Int, e);
            }
            // The mantissa: `_? H . H?`, `_? H`, or `. H`; then the exponent, which a hex float needs.
            let mut mant = Vec::with_capacity(3);
            if let Some(e) = int {
                mant.push(e);
                if self.at(e) == b'.' {
                    mant.push(self.digits(e + 1, hexd).unwrap_or(e + 1));
                }
            } else if self.at(s + 2) == b'.' {
                if let Some(f) = self.digits(s + 3, hexd) {
                    mant.push(f);
                }
            }
            for m in mant {
                if let Some(x) = self.exponent(m, true) {
                    take(T::Float, x);
                }
            }
        } else if c0 == b'0' && matches!(c1, b'b' | b'B') {
            if let Some(e) = self.prefixed_digits(s + 2, |c| c == b'0' || c == b'1') {
                take(T::Int, e);
            }
        } else if c0 == b'0' && matches!(c1, b'o' | b'O') {
            if let Some(e) = self.prefixed_digits(s + 2, |c| (b'0'..=b'7').contains(&c)) {
                take(T::Int, e);
            }
        } else {
            // Decimal: an integer (`0`, `[1-9](_?D)?`, or octal `0_?[0-7](_?[0-7])*`), or a float
            // (`D . D? exp?`, `D exp`, `. D exp?`).
            if c0 == b'0' {
                take(T::Int, s + 1);
                if let Some(e) = self.prefixed_digits(s + 1, |c| (b'0'..=b'7').contains(&c)) {
                    take(T::Int, e);
                }
            } else if c0 != b'.' {
                if let Some(e) = self.digits(s, dec) {
                    take(T::Int, e);
                }
            }
            let int = if c0 == b'.' { Some(s) } else { self.digits(s, dec) };
            if let Some(e) = int {
                if c0 != b'.' {
                    if let Some(x) = self.exponent(e, false) {
                        take(T::Float, x);
                    }
                    // `D i` (an imaginary from decimal digits alone, `0123i` included).
                    if self.at(e) == b'i' {
                        take(T::Imaginary, e + 1);
                    }
                }
                if self.at(e) == b'.' {
                    let f = self.digits(e + 1, dec);
                    if c0 != b'.' || f.is_some() {
                        let m = f.unwrap_or(e + 1);
                        take(T::Float, self.exponent(m, false).unwrap_or(m));
                    }
                }
            }
        }
        if best.1 == s {
            return Err(Refusal::Lex(s as u32));
        }
        // Any literal and `i`: an imaginary.
        if self.at(best.1) == b'i' {
            best = (T::Imaginary, best.1 + 1);
        }
        // A name's byte right after a number is no token tree-sitter reads apart.
        if block::is_word(self.at(best.1)) || self.at(best.1) == b'.' && best.0 != T::Float && self.at(best.1 + 1).is_ascii_digit() {
            return Err(Refusal::Lex(best.1 as u32));
        }
        Ok((best.0, best.1, 0))
    }

    /// The end of an interpreted string from its `"` at `s`: past its closer. `None` where a line
    /// or the file ends first.
    pub fn string_end(&self, s: usize) -> Option<usize> {
        let mut i = s + 1;
        loop {
            // The next `"`, `\` or `\n` (stage 1's mask): the text between is passed a block at a time.
            i = self.blocks.string_stop(i);
            match self.src.get(i)? {
                b'"' => return Some(i + 1),
                b'\n' => return None,
                _ => i += 2,
            }
        }
    }

    /// A rune literal from `s` (its `'`): one character but `'` and `\`, or an escape; then `'`.
    fn rune_end(&self, s: usize) -> Option<usize> {
        let i = s + 1;
        let at = |k: usize| self.at(k);
        let hex = |from: usize, n: usize| (from..from + n).all(|k| at(k).is_ascii_hexdigit());
        let end = match at(i) {
            b'\'' => return None,
            b'\\' => match at(i + 1) {
                b'x' if hex(i + 2, 2) => i + 4,
                b'0'..=b'7' if (i + 1..i + 4).all(|k| (b'0'..=b'7').contains(&at(k))) => i + 4,
                b'u' if hex(i + 2, 4) => i + 6,
                b'U' if hex(i + 2, 8) => i + 10,
                b'a' | b'b' | b'f' | b'n' | b'r' | b't' | b'v' | b'\\' | b'\'' | b'"' => i + 2,
                _ => return None,
            },
            _ if i >= self.src.len() => return None,
            c if c >= 0x80 => i + decode(self.src, i)?.1,
            _ => i + 1,
        };
        (at(end) == b'\'').then_some(end + 1)
    }

    /// An escape sequence from `i` (its `\`), as tree-sitter-go's `escape_sequence` reads one
    /// (the longest of `\` and one character but `x`, `u` and `U`; 2 or 3 digits; `x` and two hex
    /// digits; `u` and 4; `U` and 8). Its end.
    pub fn escape_end(&self, i: usize) -> Option<usize> {
        let at = |k: usize| self.at(k);
        let hex = |from: usize, n: usize| (from..from + n).all(|k| at(k).is_ascii_hexdigit());
        let c = at(i + 1);
        if i + 1 >= self.src.len() {
            return None;
        }
        match c {
            // `x[0-9a-fA-F]{2,}`, which tree-sitter's lexer stops after two digits (the content
            // token after it takes the rest).
            b'x' => hex(i + 2, 2).then_some(i + 4),
            b'u' => hex(i + 2, 4).then_some(i + 6),
            b'U' => hex(i + 2, 8).then_some(i + 10),
            b'0'..=b'9' => {
                let mut k = i + 1;
                while k < i + 4 && at(k).is_ascii_digit() {
                    k += 1;
                }
                Some(if k >= i + 3 { k } else { i + 2 })
            }
            c if c >= 0x80 => Some(i + 1 + decode(self.src, i + 1)?.1),
            _ => Some(i + 2),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Row = (T, u8, u32, u32, u32, u32);

    /// The tokens of `src` one `token` at a time (the scalar lexer), and the comments.
    fn by_token(lx: &Lexer, mut at: usize) -> (Vec<Row>, Vec<(u32, u32)>) {
        let (mut out, mut extras) = (Vec::new(), Vec::new());
        loop {
            let t = lx.token(at, &mut extras).unwrap_or_else(refused);
            out.push((t.kind, t.word, t.start, t.end, t.nl, t.nl_last));
            if t.kind == T::Eof {
                break;
            }
            at = t.end as usize;
        }
        (out, extras.iter().map(|x| (x.start, x.end)).collect())
    }

    /// The same by `run`, in batches of `n`.
    fn by_run(lx: &Lexer, mut at: usize, n: usize) -> (Vec<Row>, Vec<(u32, u32)>) {
        let (mut toks, mut extras) = (Vec::new(), Vec::new());
        loop {
            at = lx.run(at, &mut extras, &mut toks, n);
            if toks.last().is_some_and(|t| t.kind == T::Eof) {
                break;
            }
        }
        let rows = toks.iter().map(|t| (t.kind, t.word, t.start, t.end, t.nl, t.nl_last)).collect();
        (rows, extras.iter().map(|x| (x.start, x.end)).collect())
    }

    /// `run` reads what `token` reads: every token, line end, comment and refusal, over random
    /// texts of Go's pieces (and from every position of a few, as a parse may lex from anywhere).
    #[test]
    fn run_reads_as_token_does() {
        const PIECES: &[&str] = &[
            "a", "func", "x1", "_y", "0x", "0", "7", "42", "12345678", "123456789", "1.5", "1e5", "1_000", "3i", "0o7",
            " ", "  ", "\t", "\n", "\n\n", "\r\n", "/", "//c\n", "/*c*/", "/*\n*/", "(", ")", "{", "}", ";", ",", "~",
            ":", ":=", ".", "...", "<-", "<", "<<=", ">>", "=", "==", "!", "&&", "&^=", "|", "^", "%", "\"s\"",
            "\"a\\\"b\"", "'c'", "'\\n'", "`r`", "`\n`", "é", "\u{ff}", "#", "?", "\\",
        ];
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        for round in 0..3000 {
            let mut src = String::new();
            for _ in 0..(round % 90) {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                src += PIECES[(x >> 20) as usize % PIECES.len()];
            }
            let lx = Lexer::new(src.as_bytes());
            let starts: Vec<usize> = if round % 50 == 0 { (0..=src.len()).collect() } else { vec![0] };
            for at in starts {
                let want = by_token(&lx, at);
                for n in [1, 3, 16] {
                    assert_eq!(by_run(&lx, at, n), want, "{src:?} from {at}, batches of {n}");
                }
            }
        }
    }
}
