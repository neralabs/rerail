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

pub struct Lexer<'s> {
    pub src: &'s [u8],
    pub blocks: Blocks<'s>,
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

    /// The token at `s` (no white space before it): its kind, end and word code.
    fn token_at(&self, s: usize) -> R<(T, usize, u8)> {
        use T::*;
        if s >= self.src.len() {
            return Ok((Eof, s, 0));
        }
        let c = self.src[s];
        let refuse = Err(Refusal::Lex(s as u32));
        let next = self.at(s + 1);
        // An ASCII name: its end is where its word bytes stop, unless a character past ASCII goes
        // on there.
        if FIRST[usize::from(c)] & NAME == 0 {
            let end = self.blocks.word_end(s + 1);
            let end = if self.at(end) < 0x80 { end } else { self.name_end(s)? };
            return Ok((Name, end, word(self.src, s, end)));
        }
        let (kind, len) = match c {
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                let end = self.name_end(s)?;
                return Ok((Name, end, word(self.src, s, end)));
            }
            b'0'..=b'9' => return self.number(s),
            b'.' if next.is_ascii_digit() => return self.number(s),
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
            c if c >= 0x80 => {
                let end = self.name_end(s)?;
                if end == s {
                    return refuse;
                }
                return Ok((Name, end, 0));
            }
            _ => return refuse,
        };
        Ok((kind, s + len, 0))
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
            } else if self.at(s + 2) == b'.'
                && let Some(f) = self.digits(s + 3, hexd) {
                    mant.push(f);
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
            } else if c0 != b'.'
                && let Some(e) = self.digits(s, dec) {
                    take(T::Int, e);
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
