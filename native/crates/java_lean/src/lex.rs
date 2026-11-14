//! The lexer: tree-sitter-java's lexer, read where the parser asks. tree-sitter-java has no
//! external scanner. The lexer keeps no state: the parser asks for the next token at a byte
//! (`Lexer::token`), and reads itself the few places its parse state lexes otherwise (a `>>` split
//! in two in type arguments, a string's pieces).
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

/// A token's kind. Punctuation is lexed longest first, as tree-sitter's lexer does where every
/// operator is valid; the parser splits one where its state reads a shorter one (`>>` closing type
/// arguments).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum T {
    Eof,
    Name,
    Char,
    /// `decimal_integer_literal`.
    Dec,
    Hex,
    Oct,
    Bin,
    /// `decimal_floating_point_literal`.
    DecFloat,
    HexFloat,
    /// A string literal, `"` to `"` (its pieces are read by the parser from its bytes).
    Quote,
    /// A text block, `"""` to `"""`.
    TripleQuote,
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
    At,
    /// `@interface`.
    AtInterface,
    ColonColon,
    Colon,
    Question,
    Arrow,
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
    Pipe,
    PipePipe,
    PipeEq,
    Lt,
    Le,
    LtLt,
    LtLtEq,
    Gt,
    Ge,
    GtGt,
    GtGtEq,
    GtGtGt,
    GtGtGtEq,
}

#[derive(Clone, Copy, Debug)]
pub struct Tok {
    pub kind: T,
    /// A name's word code (`w::…`; 0: none).
    pub word: u8,
    pub start: u32,
    pub end: u32,
}

/// A comment (the only extra).
#[derive(Clone, Copy, Debug)]
pub struct Extra {
    pub start: u32,
    pub end: u32,
    /// A block comment (`/* */`), not a line comment.
    pub block: bool,
}

/// Word codes: the keywords, the contextual words and the primitive types.
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
        ABSTRACT = b"abstract", ASSERT = b"assert", BREAK = b"break", CASE = b"case", CATCH = b"catch",
        CLASS = b"class", CONTINUE = b"continue", DEFAULT = b"default", DO = b"do", ELSE = b"else",
        ENUM = b"enum", EXPORTS = b"exports", EXTENDS = b"extends", FALSE = b"false", FINAL = b"final",
        FINALLY = b"finally", FOR = b"for", IF = b"if", IMPLEMENTS = b"implements", IMPORT = b"import",
        INSTANCEOF = b"instanceof", INTERFACE = b"interface", MODULE = b"module", NATIVE = b"native",
        NEW = b"new", NON = b"non", NULL = b"null", OPEN = b"open", OPENS = b"opens", PACKAGE = b"package",
        PERMITS = b"permits", PRIVATE = b"private", PROTECTED = b"protected", PROVIDES = b"provides",
        PUBLIC = b"public", RECORD = b"record", REQUIRES = b"requires", RETURN = b"return", SEALED = b"sealed",
        STATIC = b"static", STRICTFP = b"strictfp", SUPER = b"super", SWITCH = b"switch",
        SYNCHRONIZED = b"synchronized", THIS = b"this", THROW = b"throw", THROWS = b"throws", TO = b"to",
        TRANSIENT = b"transient", TRANSITIVE = b"transitive", TRUE = b"true", TRY = b"try", USES = b"uses",
        VOLATILE = b"volatile", WHEN = b"when", WHILE = b"while", WITH = b"with", YIELD = b"yield",
        // The primitive types (`BYTE..=VOID`).
        BYTE = b"byte", SHORT = b"short", INT = b"int", LONG = b"long", CHAR = b"char", FLOAT = b"float",
        DOUBLE = b"double", BOOLEAN = b"boolean", VOID = b"void",
    }

    /// Whether `code` is a primitive type's word (`void` included).
    pub const fn is_primitive(code: u8) -> bool {
        code >= BYTE && code <= VOID
    }
}

/// A word's first 8 bytes as one integer, little-endian, zeros past its end.
const fn word_key(w: &[u8]) -> u64 {
    let mut k = 0u64;
    let mut i = 0;
    while i < w.len() && i < 8 {
        k |= (w[i] as u64) << (8 * i);
        i += 1;
    }
    k
}

/// A word's slot in `WORDS`, by its first 8 bytes and its length: a multiplier under which every
/// word takes a slot of its own of 1,024 (a perfect hash; `WORDS` is refused at compile time where two
/// share one).
#[inline(always)]
const fn word_slot(key: u64, len: usize) -> usize {
    ((key ^ (len as u64).wrapping_mul(0x9e37_79b9)).wrapping_mul(HASH) >> 54) as usize
}

const HASH: u64 = find_hash();

/// The first multiplier (of a fixed sequence) under which the words' slots are distinct.
const fn find_hash() -> u64 {
    let mut m = 0x8f58_640b_360e_7c81u64;
    loop {
        let mut used = [false; 1024];
        let mut ok = true;
        let mut i = 1;
        while i < w::TEXT.len() {
            let t = w::TEXT[i];
            let s = ((word_key(t) ^ (t.len() as u64).wrapping_mul(0x9e37_79b9)).wrapping_mul(m) >> 54) as usize;
            if used[s] {
                ok = false;
                break;
            }
            used[s] = true;
            i += 1;
        }
        if ok {
            return m;
        }
        m = m.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407) | 1;
    }
}

/// Each slot's word: its first 8 bytes, its length (0: none), its code.
static WORDS: [(u64, u8, u8); 1024] = {
    let mut t = [(0u64, 0u8, 0u8); 1024];
    let mut i = 1;
    while i < w::TEXT.len() {
        let text = w::TEXT[i];
        assert!(text.len() <= 16);
        let k = word_key(text);
        let slot = word_slot(k, text.len());
        assert!(t[slot].1 == 0, "two words in a slot");
        t[slot] = (k, text.len() as u8, i as u8);
        i += 1;
    }
    t
};

/// The word `src[i..end]`'s code: its first 8 bytes loaded as one integer, masked to its length,
/// hashed to its one slot and compared once (and its tail, for the few words past 8 bytes).
#[inline(always)]
pub fn word(src: &[u8], i: usize, end: usize) -> u8 {
    let len = end - i;
    if len.wrapping_sub(2) >= 15 {
        return 0;
    }
    let raw = match src.get(i..i + 8) {
        Some(b) => u64::from_le_bytes(b.try_into().expect("8 bytes")),
        None => word_key(&src[i..end]),
    };
    let k = if len >= 8 { raw } else { raw & (u64::MAX >> (64 - 8 * len)) };
    let (wk, wl, code) = WORDS[word_slot(k, len)];
    if wk != k || usize::from(wl) != len {
        return 0;
    }
    if len > 8 && w::TEXT[usize::from(code)][8..] != src[i + 8..end] {
        return 0;
    }
    code
}

/// The character at `at` (a UTF-8 lead byte at or past 0x80) and its length.
fn decode(src: &[u8], at: usize) -> Option<(char, usize)> {
    let s = std::str::from_utf8(&src[at..(at + 4).min(src.len())]).or_else(|e| std::str::from_utf8(&src[at..at + e.valid_up_to()])).ok()?;
    let c = s.chars().next()?;
    Some((c, c.len_utf8()))
}

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
    /// token starts.
    #[inline]
    pub fn skip(&self, mut at: usize, extras: &mut Vec<Extra>) -> R<usize> {
        loop {
            at = self.blocks.blank_end(at);
            match self.at(at) {
                b'\n' | b'\r' => at += 1,
                b'/' if self.at(at + 1) == b'/' => {
                    // To the `\n` (a `\r` is the comment's text: `[^\n]*` reads it).
                    let mut end = self.blocks.eol(at + 2);
                    while end < self.src.len() && self.src[end] != b'\n' {
                        end = self.blocks.eol(end + 1);
                    }
                    extras.push(Extra { start: at as u32, end: end as u32, block: false });
                    at = end;
                }
                b'/' if self.at(at + 1) == b'*' => {
                    let mut i = at + 2;
                    loop {
                        i = self.blocks.star(i);
                        if i >= self.src.len() {
                            return Err(Refusal::Lex(at as u32));
                        }
                        if self.src[i] == b'*' && self.at(i + 1) == b'/' {
                            i += 2;
                            break;
                        }
                        i += 1;
                    }
                    extras.push(Extra { start: at as u32, end: i as u32, block: true });
                    at = i;
                }
                c if c >= 0x80 => match decode(self.src, at) {
                    Some((c, n)) if c.is_whitespace() => at += n,
                    _ => return Ok(at),
                },
                _ => return Ok(at),
            }
        }
    }

    /// The next token from `at`, white space and comments passed.
    #[inline]
    pub fn token(&self, at: usize, extras: &mut Vec<Extra>) -> R<Tok> {
        let s = self.skip(at, extras)?;
        let (kind, end, word) = self.token_at(s)?;
        Ok(Tok { kind, word, start: s as u32, end: end as u32 })
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
        let (kind, len) = match c {
            b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'$' => {
                let end = self.name_end(s)?;
                return Ok((Name, end, word(self.src, s, end)));
            }
            b'0'..=b'9' => return self.number(s),
            b'.' if next.is_ascii_digit() => return self.number(s),
            b'"' => {
                let block = next == b'"' && self.at(s + 2) == b'"';
                return match self.string_end(s, block) {
                    Some(end) => Ok((if block { TripleQuote } else { Quote }, end, 0)),
                    None => refuse,
                };
            }
            b'\'' => match self.char_end(s) {
                Some(end) => return Ok((Char, end, 0)),
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
            b'?' => (Question, 1),
            b'~' => (Tilde, 1),
            b'@' => {
                if self.src[s + 1..].starts_with(b"interface") {
                    (AtInterface, 10)
                } else {
                    (At, 1)
                }
            }
            b':' => {
                if next == b':' {
                    (ColonColon, 2)
                } else {
                    (Colon, 1)
                }
            }
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
                b'>' => (Arrow, 2),
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
            b'&' => match next {
                b'&' => (AmpAmp, 2),
                b'=' => (AmpEq, 2),
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
                _ => (Lt, 1),
            },
            b'>' => match (next, self.at(s + 2), self.at(s + 3)) {
                (b'>', b'>', b'=') => (GtGtGtEq, 4),
                (b'>', b'>', _) => (GtGtGt, 3),
                (b'>', b'=', _) => (GtGtEq, 3),
                (b'>', _, _) => (GtGt, 2),
                (b'=', _, _) => (Ge, 2),
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

    /// The end of a name from `s` (`[\p{XID_Start}_$][\p{XID_Continue}¢_$]*`): ASCII word
    /// bytes a block a step, a character past ASCII by its class. `s` itself where no name starts
    /// there.
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
            let ok = if i == s { unicode_ident::is_xid_start(ch) } else { unicode_ident::is_xid_continue(ch) || ch == '\u{a2}' };
            if !ok {
                return Ok(i);
            }
            i += n;
        }
    }

    /// A run of `digit` bytes with single `_` between them (`sep1(/[digits]+/, '_')`) from `i`:
    /// its end, `None` where no digit starts it. A `_` not followed by a digit ends the run before it.
    fn digits(&self, mut i: usize, digit: impl Fn(u8) -> bool) -> Option<usize> {
        if !digit(self.at(i)) {
            return None;
        }
        loop {
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

    /// `DIGITS`: `0`, or `[1-9]` then digits with runs of `_` between them. Its end.
    fn digits_dec(&self, s: usize) -> Option<usize> {
        match self.at(s) {
            b'0' => Some(s + 1),
            b'1'..=b'9' => {
                let mut i = s + 1;
                loop {
                    while self.at(i).is_ascii_digit() {
                        i += 1;
                    }
                    let mut k = i;
                    while self.at(k) == b'_' {
                        k += 1;
                    }
                    if k > i && self.at(k).is_ascii_digit() {
                        i = k;
                        continue;
                    }
                    return Some(i);
                }
            }
            _ => None,
        }
    }

    /// An exponent from `i` (`[eE][-+]?DECIMAL_DIGITS`, or `[pP][-+]?DIGITS` where `hex`): its end.
    fn exponent(&self, i: usize, hex: bool) -> Option<usize> {
        let c = self.at(i);
        if !(if hex { matches!(c, b'p' | b'P') } else { matches!(c, b'e' | b'E') }) {
            return None;
        }
        let k = if matches!(self.at(i + 1), b'+' | b'-') { i + 2 } else { i + 1 };
        if hex { self.digits_dec(k) } else { self.digits(k, |c| c.is_ascii_digit()) }
    }

    /// A number at `s` (a digit, or a `.` before one): the longest of the grammar's literals.
    fn number(&self, s: usize) -> R<(T, usize, u8)> {
        let suffix = |i: usize, set: &[u8]| if set.contains(&self.at(i)) { i + 1 } else { i };
        let float_suffix = |i: usize| suffix(i, b"fFdD");
        let long_suffix = |i: usize| suffix(i, b"lL");
        let mut best = (T::Dec, s);
        let mut take = |kind: T, end: usize| {
            if end > best.1 {
                best = (kind, end);
            }
        };
        let dec = |c: u8| c.is_ascii_digit();
        let hexd = |c: u8| c.is_ascii_hexdigit();
        let (c0, c1) = (self.at(s), self.at(s + 1));
        if c0 == b'0' && matches!(c1, b'x' | b'X') {
            // Hex integers and floats.
            let int = self.digits(s + 2, hexd);
            if let Some(e) = int {
                take(T::Hex, long_suffix(e));
            }
            // `HEX_DIGITS '.'?` or `HEX_DIGITS? '.' HEX_DIGITS`, then an optional exponent.
            let mut mant = Vec::new();
            if let Some(e) = int {
                mant.push(e);
                if self.at(e) == b'.' {
                    mant.push(e + 1);
                    if let Some(f) = self.digits(e + 1, hexd) {
                        mant.push(f);
                    }
                }
            } else if self.at(s + 2) == b'.' {
                if let Some(f) = self.digits(s + 3, hexd) {
                    mant.push(f);
                }
            }
            for m in mant {
                // Without an exponent: a hex float only where it differs from the integer (a `.`).
                if Some(m) != int {
                    take(T::HexFloat, m);
                }
                if let Some(x) = self.exponent(m, true) {
                    take(T::HexFloat, float_suffix(x));
                }
            }
            return Ok((best.0, best.1, 0));
        }
        if c0 == b'0' && matches!(c1, b'b' | b'B') {
            if let Some(e) = self.digits(s + 2, |c| c == b'0' || c == b'1') {
                take(T::Bin, long_suffix(e));
            }
        }
        if c0 == b'0' && matches!(c1, b'o' | b'O') {
            if let Some(e) = self.digits(s + 2, |c| (b'0'..=b'7').contains(&c)) {
                take(T::Oct, long_suffix(e));
            }
        }
        if c0 == b'0' {
            if let Some(e) = self.digits(s + 1, |c| (b'0'..=b'7').contains(&c)) {
                take(T::Oct, long_suffix(e));
            }
        }
        if let Some(e) = self.digits_dec(s) {
            take(T::Dec, long_suffix(e));
            // `DIGITS exponent suffix?` and `DIGITS exponent? suffix`.
            if let Some(x) = self.exponent(e, false) {
                take(T::DecFloat, float_suffix(x));
            }
            if matches!(self.at(e), b'f' | b'F' | b'd' | b'D') {
                take(T::DecFloat, e + 1);
            }
        }
        // `DECIMAL_DIGITS '.' DECIMAL_DIGITS? exponent? suffix?`, and `'.' DECIMAL_DIGITS …`.
        let int = if c0 == b'.' { Some(s) } else { self.digits(s, dec) };
        if let Some(e) = int {
            if self.at(e) == b'.' {
                let f = self.digits(e + 1, dec);
                if c0 != b'.' || f.is_some() {
                    let m = f.unwrap_or(e + 1);
                    let x = self.exponent(m, false).unwrap_or(m);
                    take(T::DecFloat, float_suffix(x));
                }
            }
        }
        if best.1 == s {
            return Err(Refusal::Lex(s as u32));
        }
        Ok((best.0, best.1, 0))
    }

    /// The end of a string literal from its `"` at `s` (a text block's `"""` where `block`): past
    /// its closer. An interpolation's (`\{…}`) brackets and strings are passed. `None` where the
    /// file ends first.
    pub fn string_end(&self, s: usize, block: bool) -> Option<usize> {
        let mut i = if block { s + 3 } else { s + 1 };
        loop {
            match self.src.get(i)? {
                b'"' if !block => return Some(i + 1),
                b'"' if self.src[i..].starts_with(b"\"\"\"") => return Some(i + 3),
                b'\\' if self.at(i + 1) == b'{' => i = self.interpolation_end(i + 2)?,
                b'\\' => i += 2,
                _ => i += 1,
            }
        }
    }

    /// From inside an interpolation (past its `\{`): past its `}`, nested brackets, strings and
    /// characters passed.
    fn interpolation_end(&self, mut i: usize) -> Option<usize> {
        let mut depth = 0u32;
        loop {
            match self.src.get(i)? {
                b'{' => depth += 1,
                b'}' if depth == 0 => return Some(i + 1),
                b'}' => depth -= 1,
                b'"' => {
                    let block = self.at(i + 1) == b'"' && self.at(i + 2) == b'"';
                    i = self.string_end(i, block)?;
                    continue;
                }
                b'\'' => {
                    i = self.char_end(i)?;
                    continue;
                }
                _ => {}
            }
            i += 1;
        }
    }

    /// A character literal from `s` (its `'`): `'` then one or more of a character but `\`, `'`
    /// and a line end, `\` and any character, or `\` and a line end; then `'`. Its end.
    fn char_end(&self, s: usize) -> Option<usize> {
        let mut i = s + 1;
        let mut n = 0;
        loop {
            match self.at(i) {
                b'\'' if n > 0 => return Some(i + 1),
                b'\'' | b'\n' => return None,
                _ if i >= self.src.len() => return None,
                b'\\' => {
                    let c = self.at(i + 1);
                    if i + 1 >= self.src.len() {
                        return None;
                    }
                    i += if c >= 0x80 { 1 + decode(self.src, i + 1)?.1 } else { 2 };
                }
                c if c >= 0x80 => i += decode(self.src, i)?.1,
                _ => i += 1,
            }
            n += 1;
        }
    }

    /// An escape sequence from `i` (its `\`): `\` then any character but `x`, `u` and an octal
    /// digit; 1 to 3 octal digits; `x` and 2 hex digits; `u` and 4; or `u{…}`. Its end.
    pub fn escape_end(&self, i: usize) -> Option<usize> {
        let at = |k: usize| self.at(k);
        match at(i + 1) {
            b'u' => {
                if at(i + 2) == b'{' {
                    let mut k = i + 3;
                    while at(k).is_ascii_hexdigit() {
                        k += 1;
                    }
                    (k > i + 3 && at(k) == b'}').then_some(k + 1)
                } else {
                    (i + 2..i + 6).all(|k| at(k).is_ascii_hexdigit()).then_some(i + 6)
                }
            }
            b'x' => (at(i + 2).is_ascii_hexdigit() && at(i + 3).is_ascii_hexdigit()).then_some(i + 4),
            b'0'..=b'7' => {
                let mut k = i + 2;
                while k < i + 4 && (b'0'..=b'7').contains(&at(k)) {
                    k += 1;
                }
                Some(k)
            }
            _ if i + 1 >= self.src.len() => None,
            c if c >= 0x80 => Some(i + 1 + decode(self.src, i + 1)?.1),
            _ => Some(i + 2),
        }
    }
}
