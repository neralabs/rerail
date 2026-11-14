//! The lexer: tree-sitter-javascript's (and -typescript's) lexer and external scanner, read where
//! the parser asks. tree-sitter lexes by parse state: a `/` is a regular expression where an
//! operand may start and a division elsewhere, a template's text and JSX text are tokens only inside
//! a template or an element. So the lexer keeps no state: the parser asks for the next token at a
//! byte, in the mode its state reads (`Lexer::token`, `template_piece`, `regex`, `jsx_text`, ...).
//!
//! Every word is a `Name`, with its word code (`w::…`; 0: none) from one hash probe: tree-sitter
//! reads a keyword only where its parse state takes it (its keyword extraction, with no reserved
//! words), so the parser decides.
//!
//! Comments are extras, in a list of their own, written where the parser next opens a node or reads
//! a token (as in `rerail-python-lean`).

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

/// A token's kind (normal mode; the other modes' pieces are read apart).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum T {
    Eof,
    Name,
    /// `#name`.
    Private,
    Number,
    /// A whole string, its quotes included (its pieces are read by `string_pieces`).
    Str,
    /// A template's opening `` ` ``: its pieces follow in template mode.
    Backtick,
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Semi,
    Comma,
    Dot,
    Ellipsis,
    QDot,
    Question,
    QQ,
    Colon,
    Arrow,
    At,
    Lt,
    Gt,
    Le,
    Ge,
    EqEq,
    Ne,
    EqEqEq,
    NeEq,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    StarStar,
    PlusPlus,
    MinusMinus,
    LtLt,
    GtGt,
    GtGtGt,
    Amp,
    Pipe,
    Caret,
    Bang,
    Tilde,
    AmpAmp,
    PipePipe,
    Eq,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    StarStarEq,
    LtLtEq,
    GtGtEq,
    GtGtGtEq,
    AmpEq,
    PipeEq,
    CaretEq,
    AmpAmpEq,
    PipePipeEq,
    QQEq,
}

#[derive(Clone, Copy, Debug)]
pub struct Tok {
    pub kind: T,
    /// A name's word code (`w::…`; 0: none).
    pub word: u8,
    /// Whether a line terminator stands between the previous token and this one (in white space or
    /// a block comment).
    pub nl: bool,
    pub start: u32,
    pub end: u32,
}

/// A comment (the only extra but HTML comments, which are refused).
#[derive(Clone, Copy, Debug)]
pub struct Extra {
    pub start: u32,
    pub end: u32,
}

/// Word codes: the keywords and contextual words of JavaScript and TypeScript.
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
        BREAK = b"break", CASE = b"case", CATCH = b"catch", CLASS = b"class", CONST = b"const",
        CONTINUE = b"continue", DEBUGGER = b"debugger", DEFAULT = b"default", DELETE = b"delete", DO = b"do",
        ELSE = b"else", EXPORT = b"export", EXTENDS = b"extends", FALSE = b"false", FINALLY = b"finally",
        FOR = b"for", FUNCTION = b"function", IF = b"if", IMPORT = b"import", IN = b"in",
        INSTANCEOF = b"instanceof", NEW = b"new", NULL = b"null", RETURN = b"return", SUPER = b"super",
        SWITCH = b"switch", THIS = b"this", THROW = b"throw", TRUE = b"true", TRY = b"try",
        TYPEOF = b"typeof", VAR = b"var", VOID = b"void", WHILE = b"while", WITH = b"with",
        YIELD = b"yield", LET = b"let", STATIC = b"static", ASYNC = b"async", AWAIT = b"await",
        GET = b"get", SET = b"set", OF = b"of", FROM = b"from", AS = b"as",
        TARGET = b"target", META = b"meta", UNDEFINED = b"undefined", USING = b"using",
        // TypeScript's.
        ABSTRACT = b"abstract", ACCESSOR = b"accessor", ANY = b"any", ASSERTS = b"asserts", BOOLEAN = b"boolean",
        DECLARE = b"declare", ENUM = b"enum", GLOBAL = b"global", IMPLEMENTS = b"implements", INFER = b"infer",
        INTERFACE = b"interface", IS = b"is", KEYOF = b"keyof", MODULE = b"module", NAMESPACE = b"namespace",
        NEVER = b"never", NUMBER = b"number", OBJECT = b"object", OVERRIDE = b"override", PRIVATE = b"private",
        PROTECTED = b"protected", PUBLIC = b"public", READONLY = b"readonly", REQUIRE = b"require", SATISFIES = b"satisfies",
        STRING = b"string", SYMBOL = b"symbol", TYPE = b"type", UNIQUE = b"unique", UNKNOWN = b"unknown",
        OUT = b"out", ASSERT = b"assert", BIGINT = b"bigint",
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
    if len > 8 && &w::TEXT[usize::from(code)][8..] != &src[i + 8..end] {
        return 0;
    }
    code
}

/// A piece of a template, read in template mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Piece {
    /// Text (`string_fragment`).
    Chars,
    Escape,
    /// `${`.
    Open,
    /// The closing `` ` ``.
    Close,
}

/// White space as the grammar's extras read it (`\s`, `\p{Zs}`, U+FEFF, U+2028, U+2029, U+2060,
/// U+200B), from a byte at or past 0x80: its length, or 0 (not white space).
fn wide_space(src: &[u8], at: usize) -> (usize, bool) {
    let c = match decode(src, at) {
        Some(c) => c,
        None => return (0, false),
    };
    let space = matches!(c.0, 0x85 | 0xa0 | 0x1680 | 0x2000..=0x200b | 0x2028 | 0x2029 | 0x202f | 0x205f | 0x2060 | 0x3000 | 0xfeff);
    if space { (c.1, matches!(c.0, 0x2028 | 0x2029)) } else { (0, false) }
}

/// The character at `at` (a UTF-8 lead byte at or past 0x80) and its length.
fn decode(src: &[u8], at: usize) -> Option<(u32, usize)> {
    let s = std::str::from_utf8(&src[at..(at + 4).min(src.len())]).or_else(|e| std::str::from_utf8(&src[at..at + e.valid_up_to()])).ok()?;
    let c = s.chars().next()?;
    Some((c as u32, c.len_utf8()))
}

pub struct Lexer<'s> {
    pub src: &'s [u8],
    blocks: Blocks<'s>,
}

impl<'s> Lexer<'s> {
    pub fn new(src: &'s [u8]) -> Self {
        Lexer { src, blocks: Blocks::of(src) }
    }

    #[inline(always)]
    fn at(&self, i: usize) -> u8 {
        self.src.get(i).copied().unwrap_or(0)
    }

    /// From `at`: white space and comments passed (the comments pushed to `extras`); where the next
    /// token starts, and whether a line terminator was passed.
    #[inline]
    pub fn skip(&self, mut at: usize, extras: &mut Vec<Extra>) -> R<(usize, bool)> {
        let mut nl = false;
        loop {
            at = self.blocks.blank_end(at);
            match self.at(at) {
                b'\n' | b'\r' => {
                    nl = true;
                    at += 1;
                }
                b'/' if self.at(at + 1) == b'/' => {
                    let end = self.line_comment_end(at + 2);
                    extras.push(Extra { start: at as u32, end: end as u32 });
                    at = end;
                }
                b'/' if self.at(at + 1) == b'*' => {
                    let (end, has_nl) = self.block_comment_end(at)?;
                    nl |= has_nl;
                    extras.push(Extra { start: at as u32, end: end as u32 });
                    at = end;
                }
                c if c >= 0x80 => {
                    let (len, line) = wide_space(self.src, at);
                    if len == 0 {
                        return Ok((at, nl));
                    }
                    nl |= line;
                    at += len;
                }
                _ => return Ok((at, nl)),
            }
        }
    }

    /// A `//` comment's end: the next line terminator (LF, CR, U+2028, U+2029) or the text's end.
    fn line_comment_end(&self, from: usize) -> usize {
        let end = self.blocks.eol(from);
        // U+2028 and U+2029 (E2 80 A8, E2 80 A9) end it too.
        match memchr_e2(&self.src[from..end]) {
            Some(k) => {
                let mut i = from + k;
                while i < end {
                    if self.src[i] == 0xe2 && self.at(i + 1) == 0x80 && matches!(self.at(i + 2), 0xa8 | 0xa9) {
                        return i;
                    }
                    i += 1 + memchr_e2(&self.src[i + 1..end]).unwrap_or(end);
                }
                end
            }
            None => end,
        }
    }

    /// A `/*` comment from `at`: its end, and whether it holds a line terminator.
    fn block_comment_end(&self, at: usize) -> R<(usize, bool)> {
        let mut i = at + 2;
        let mut nl = false;
        loop {
            i = self.blocks.star(i);
            match self.at(i) {
                _ if i >= self.src.len() => return Err(Refusal::Lex(at as u32)),
                b'*' if self.at(i + 1) == b'/' => return Ok((i + 2, nl || self.wide_line_in(at, i))),
                b'*' => i += 1,
                _ => {
                    nl = true;
                    i += 1;
                }
            }
        }
    }

    /// Whether `src[from..to]` holds U+2028 or U+2029.
    fn wide_line_in(&self, from: usize, to: usize) -> bool {
        self.src[from..to].windows(3).any(|w| w[0] == 0xe2 && w[1] == 0x80 && matches!(w[2], 0xa8 | 0xa9))
    }

    /// Whether only blanks stand before `s` on its line, or a comment's end (`/`).
    fn line_start_before(&self, s: usize) -> bool {
        let before = self.src[..s].iter().rposition(|&c| c != b' ' && c != b'\t');
        before.is_none_or(|i| matches!(self.src[i], b'\n' | b'\r' | b'/'))
    }

    /// The next token from `at` in normal mode, white space and comments passed.
    pub fn token(&self, at: usize, extras: &mut Vec<Extra>) -> R<Tok> {
        let (s, nl) = self.skip(at, extras)?;
        let (kind, end, word) = self.token_at(s)?;
        Ok(Tok { kind, word, nl, start: s as u32, end: end as u32 })
    }

    /// The token at `s` (no white space before it): its kind, end and word code.
    fn token_at(&self, s: usize) -> R<(T, usize, u8)> {
        use T::*;
        let c = self.at(s);
        if s >= self.src.len() {
            return Ok((Eof, s, 0));
        }
        let refuse = Err(Refusal::Lex(s as u32));
        let next = self.at(s + 1);
        let (kind, len) = match c {
            b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'$' => {
                let end = self.name_end(s)?;
                return Ok((Name, end, word(self.src, s, end)));
            }
            b'#' => {
                let end = self.name_end(s + 1)?;
                if end == s + 1 || self.at(s + 1).is_ascii_digit() {
                    return refuse;
                }
                return Ok((Private, end, 0));
            }
            b'0'..=b'9' => return Ok((Number, self.number_end(s)?, 0)),
            b'.' if next.is_ascii_digit() => return Ok((Number, self.number_end(s)?, 0)),
            b'"' | b'\'' => return Ok((Str, self.string_end(s)?, 0)),
            b'`' => (Backtick, 1),
            b'{' => (LBrace, 1),
            b'}' => (RBrace, 1),
            b'(' => (LParen, 1),
            b')' => (RParen, 1),
            b'[' => (LBracket, 1),
            b']' => (RBracket, 1),
            b';' => (Semi, 1),
            b',' => (Comma, 1),
            b':' => (Colon, 1),
            b'~' => (Tilde, 1),
            b'@' => (At, 1),
            b'.' => {
                if next == b'.' && self.at(s + 2) == b'.' {
                    (Ellipsis, 3)
                } else {
                    (Dot, 1)
                }
            }
            b'?' => match next {
                b'.' if !self.at(s + 2).is_ascii_digit() => (QDot, 2),
                b'?' if self.at(s + 2) == b'=' => (QQEq, 3),
                b'?' => (QQ, 2),
                _ => (Question, 1),
            },
            b'=' => match (next, self.at(s + 2)) {
                (b'=', b'=') => (EqEqEq, 3),
                (b'=', _) => (EqEq, 2),
                (b'>', _) => (Arrow, 2),
                _ => (Eq, 1),
            },
            b'!' => match (next, self.at(s + 2)) {
                (b'=', b'=') => (NeEq, 3),
                (b'=', _) => (Ne, 2),
                _ => (Bang, 1),
            },
            b'+' => match next {
                b'+' => (PlusPlus, 2),
                b'=' => (PlusEq, 2),
                _ => (Plus, 1),
            },
            b'-' => match next {
                // `-->` at a line's start (or after a comment): an HTML comment; refused. After
                // a token on its line, `--` (`a-->0`).
                b'-' if self.at(s + 2) == b'>' && self.line_start_before(s) => return refuse,
                b'-' => (MinusMinus, 2),
                b'=' => (MinusEq, 2),
                _ => (Minus, 1),
            },
            b'*' => match (next, self.at(s + 2)) {
                (b'*', b'=') => (StarStarEq, 3),
                (b'*', _) => (StarStar, 2),
                (b'=', _) => (StarEq, 2),
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
            b'<' => match (next, self.at(s + 2)) {
                // `<!--`: an HTML comment; refused.
                (b'!', b'-') => return refuse,
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
            b'&' => match (next, self.at(s + 2)) {
                (b'&', b'=') => (AmpAmpEq, 3),
                (b'&', _) => (AmpAmp, 2),
                (b'=', _) => (AmpEq, 2),
                _ => (Amp, 1),
            },
            b'|' => match (next, self.at(s + 2)) {
                (b'|', b'=') => (PipePipeEq, 3),
                (b'|', _) => (PipePipe, 2),
                (b'=', _) => (PipeEq, 2),
                _ => (Pipe, 1),
            },
            b'^' => match next {
                b'=' => (CaretEq, 2),
                _ => (Caret, 1),
            },
            b'\\' => return refuse,
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

    /// The end of a name from `s`: ASCII word bytes and any character past ASCII that is not white
    /// space (the grammar's `identifier`). An escape (`a`) is refused.
    #[inline]
    pub fn name_end(&self, s: usize) -> R<usize> {
        let mut i = self.blocks.word_end(s);
        loop {
            match self.at(i) {
                c if c >= 0x80 && i < self.src.len() => {
                    let (len, _) = wide_space(self.src, i);
                    if len > 0 {
                        return Ok(i);
                    }
                    let Some((_, n)) = decode(self.src, i) else { return Err(Refusal::Lex(i as u32)) };
                    i = self.blocks.word_end(i + n);
                }
                b'\\' => return Err(Refusal::Lex(i as u32)),
                _ => return Ok(i),
            }
        }
    }

    /// A number's end from `s`, as the grammar's `number` token reads it; refused where its forms
    /// overlap (a leading zero before a digit and a `.` or an exponent).
    fn number_end(&self, s: usize) -> R<usize> {
        let refuse = Err(Refusal::Lex(s as u32));
        let digits = |mut i: usize, ok: &dyn Fn(u8) -> bool| -> usize {
            // `d(_?d)*`
            if !ok(self.at(i)) {
                return i;
            }
            i += 1;
            loop {
                if ok(self.at(i)) {
                    i += 1;
                } else if self.at(i) == b'_' && ok(self.at(i + 1)) {
                    i += 2;
                } else {
                    return i;
                }
            }
        };
        let dec = |c: u8| c.is_ascii_digit();
        let c = self.at(s);
        let n = self.at(s + 1);
        let mut i;
        if c == b'0' && matches!(n, b'x' | b'X' | b'b' | b'B' | b'o' | b'O') {
            let ok: &dyn Fn(u8) -> bool = match n {
                b'x' | b'X' => &|c: u8| c.is_ascii_hexdigit(),
                b'b' | b'B' => &|c: u8| matches!(c, b'0' | b'1'),
                _ => &|c: u8| matches!(c, b'0'..=b'7'),
            };
            i = digits(s + 2, ok);
            if i == s + 2 {
                return refuse;
            }
            if self.at(i) == b'n' {
                i += 1;
            }
        } else {
            let leading_zero = c == b'0' && dec(n);
            i = if c == b'.' { s } else { digits(s, &dec) };
            let mut fraction = false;
            if self.at(i) == b'.' {
                fraction = true;
                i = digits(i + 1, &dec);
            }
            let mut exp = false;
            if matches!(self.at(i), b'e' | b'E') {
                let mut j = i + 1;
                if matches!(self.at(j), b'+' | b'-') {
                    j += 1;
                }
                let k = digits(j, &dec);
                if k > j {
                    i = k;
                    exp = true;
                }
            }
            if leading_zero && (fraction || exp || self.src[s..i].contains(&b'_')) {
                return refuse;
            }
            if !fraction && !exp && self.at(i) == b'n' {
                i += 1;
            }
        }
        // A name's byte right after a number: tree-sitter reads two tokens; refused.
        if block::is_word(self.at(i)) || self.at(i) >= 0x80 {
            return refuse;
        }
        Ok(i)
    }

    /// A string's end from its quote at `s`: past its closing quote. A line end inside it (but
    /// after `\`) is refused, as is a string the text ends in.
    fn string_end(&self, s: usize) -> R<usize> {
        let q = self.src[s];
        let mut i = s + 1;
        loop {
            i = self.blocks.string_stop(i);
            match self.at(i) {
                _ if i >= self.src.len() => return Err(Refusal::Lex(s as u32)),
                c if c == q => return Ok(i + 1),
                b'\\' => i += if self.at(i + 1) == b'\r' && self.at(i + 2) == b'\n' { 3 } else { 2 },
                b'\n' | b'\r' => return Err(Refusal::Lex(i as u32)),
                _ => i += 1,
            }
        }
    }

    /// An escape sequence's end from its `\` at `s`, as the grammar's `escape_sequence` reads it;
    /// refused where it reads none (`\x` without two hex digits).
    pub fn escape_end(&self, s: usize) -> R<usize> {
        let c = self.at(s + 1);
        let hex = |i: usize| self.at(i).is_ascii_hexdigit();
        Ok(match c {
            b'x' if hex(s + 2) && hex(s + 3) => s + 4,
            b'u' if (2..6).all(|k| hex(s + k)) => s + 6,
            b'u' if self.at(s + 2) == b'{' => {
                let mut i = s + 3;
                while hex(i) {
                    i += 1;
                }
                if i == s + 3 || self.at(i) != b'}' {
                    return Err(Refusal::Lex(s as u32));
                }
                i + 1
            }
            b'x' | b'u' => return Err(Refusal::Lex(s as u32)),
            b'0'..=b'7' => {
                let mut i = s + 2;
                while i < s + 4 && matches!(self.at(i), b'0'..=b'7') {
                    i += 1;
                }
                i
            }
            b'\r' if self.at(s + 2) == b'\n' => s + 3,
            _ if s + 1 >= self.src.len() => return Err(Refusal::Lex(s as u32)),
            c if c >= 0x80 => match decode(self.src, s + 1) {
                Some((_, n)) => s + 1 + n,
                None => return Err(Refusal::Lex(s as u32)),
            },
            _ => s + 2,
        })
    }

    /// The pieces of a string token `s..e` (quotes included): `(start, end, escape)` each.
    pub fn string_pieces(&self, s: usize, e: usize, out: &mut Vec<(u32, u32, bool)>) -> R<()> {
        let mut i = s + 1;
        let end = e - 1;
        while i < end {
            if self.src[i] == b'\\' {
                let x = self.escape_end(i)?;
                out.push((i as u32, x as u32, true));
                i = x;
                continue;
            }
            let mut j = i;
            loop {
                j = self.blocks.string_stop(j).min(end);
                if j >= end || self.src[j] == b'\\' {
                    break;
                }
                j += 1;
            }
            out.push((i as u32, j as u32, false));
            i = j;
        }
        Ok(())
    }

    /// A template's piece at `at` (template mode: no white space is passed): its kind and end.
    pub fn template_piece(&self, at: usize) -> R<(Piece, usize)> {
        match self.at(at) {
            _ if at >= self.src.len() => Err(Refusal::Lex(at as u32)),
            b'`' => Ok((Piece::Close, at + 1)),
            b'\\' => Ok((Piece::Escape, self.escape_end(at)?)),
            b'$' if self.at(at + 1) == b'{' => Ok((Piece::Open, at + 2)),
            _ => {
                let mut i = at;
                loop {
                    i = self.blocks.string_stop(i);
                    match self.at(i) {
                        _ if i >= self.src.len() => return Err(Refusal::Lex(at as u32)),
                        b'`' | b'\\' => return Ok((Piece::Chars, i)),
                        b'$' if self.at(i + 1) == b'{' => return Ok((Piece::Chars, i)),
                        _ => i += 1,
                    }
                }
            }
        }
    }

    /// A regular expression from its `/` at `s`: the pattern's end and the flags' end (the same
    /// where it has none).
    pub fn regex(&self, s: usize) -> R<(usize, usize)> {
        let refuse = Err(Refusal::Lex(s as u32));
        let mut i = s + 1;
        let mut class = false;
        loop {
            i = self.blocks.regex_stop(i);
            match self.at(i) {
                _ if i >= self.src.len() => return refuse,
                b'\n' => return refuse,
                b'\r' => i += 1,
                b'\\' => {
                    if matches!(self.at(i + 1), b'\n') || i + 1 >= self.src.len() {
                        return refuse;
                    }
                    i += 2;
                }
                b'[' => {
                    class = true;
                    i += 1;
                }
                b']' => {
                    class = false;
                    i += 1;
                }
                b'/' if class => i += 1,
                _ => break,
            }
        }
        if i == s + 1 || class {
            return refuse;
        }
        let pattern_end = i;
        let mut f = i + 1;
        while self.at(f).is_ascii_lowercase() {
            f += 1;
        }
        Ok((pattern_end, f))
    }

    /// JSX text at `at`, as the scanner reads it: its end, where it holds text (else `None`: white
    /// space alone, on lines of its own).
    pub fn jsx_text(&self, at: usize) -> Option<usize> {
        let mut saw_text = false;
        let mut at_newline = false;
        let mut i = at;
        loop {
            let stop = self.blocks.jsx_stop(i);
            // The bytes before the stop: text, unless white space after a line end.
            for &c in &self.src[i..stop] {
                let ws = c == b' ' || (b'\t'..=b'\r').contains(&c);
                at_newline &= ws;
                if !at_newline {
                    saw_text = true;
                }
            }
            i = stop;
            match self.at(i) {
                _ if i >= self.src.len() => break,
                b'\n' => {
                    at_newline = true;
                    i += 1;
                }
                b'&' => {
                    if self.character_reference(i).is_some() {
                        return saw_text.then_some(i);
                    }
                    saw_text = true;
                    at_newline = false;
                    i += 1;
                }
                _ => break,
            }
        }
        saw_text.then_some(i)
    }

    /// An HTML character reference at `s` (`&amp;`, `&#38;`, `&#x26;`): its end.
    pub fn character_reference(&self, s: usize) -> Option<usize> {
        let mut i = s + 1;
        let (max, ok): (usize, fn(u8) -> bool) = match (self.at(i), self.at(i + 1)) {
            (b'#', b'x' | b'X') => {
                i += 2;
                (6, |c| c.is_ascii_hexdigit())
            }
            (b'#', _) => {
                i += 1;
                (5, |c| c.is_ascii_digit())
            }
            _ => (30, |c| c.is_ascii_alphabetic()),
        };
        let from = i;
        while i - from < max && ok(self.at(i)) {
            i += 1;
        }
        (i > from && self.at(i) == b';').then_some(i + 1)
    }

    /// A JSX name's end from `s`: `jsx_identifier` (`[a-zA-Z_$][a-zA-Z0-9_$]*-[a-zA-Z0-9_$-]*`)
    /// where a `-` follows the word, else an identifier's. Whether it was a `jsx_identifier`.
    pub fn jsx_name_end(&self, s: usize) -> R<(usize, bool)> {
        let c = self.at(s);
        if !(c.is_ascii_alphabetic() || c == b'_' || c == b'$') {
            return Ok((self.name_end(s)?, false));
        }
        let w = self.blocks.word_end(s);
        if self.at(w) != b'-' {
            return Ok((self.name_end(s)?, false));
        }
        let mut i = w;
        while block::is_word(self.at(i)) || self.at(i) == b'-' {
            i += 1;
        }
        if self.at(i) >= 0x80 || self.at(i) == b'\\' {
            return Err(Refusal::Lex(i as u32));
        }
        Ok((i, true))
    }
}

/// The first 0xE2 byte in `b`.
fn memchr_e2(b: &[u8]) -> Option<usize> {
    b.iter().position(|&c| c == 0xe2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_are_found_by_their_one_slot() {
        for (code, &w) in w::TEXT.iter().enumerate().skip(1) {
            for text in [w.to_vec(), [w, b" + xyzzy_long_tail"].concat()] {
                assert_eq!(word(&text, 0, w.len()), code as u8, "{}", String::from_utf8_lossy(w));
            }
            let longer = [w, b"x"].concat();
            assert_eq!(word(&longer, 0, longer.len()), 0, "{}", String::from_utf8_lossy(&longer));
            let shorter = &w[..w.len() - 1];
            assert_eq!(word(shorter, 0, shorter.len()), w::TEXT.iter().position(|t| *t == shorter).unwrap_or(0) as u8);
        }
        assert_eq!(word(b"instanceof", 0, 10), w::INSTANCEOF);
        assert_eq!(word(b"instanceog", 0, 10), 0);
        assert_eq!(word(b"x_return", 2, 8), w::RETURN);
    }

    fn kinds(src: &str) -> Vec<T> {
        let l = Lexer::new(src.as_bytes());
        let mut extras = Vec::new();
        let mut at = 0;
        let mut out = Vec::new();
        loop {
            let t = l.token(at, &mut extras).unwrap();
            out.push(t.kind);
            if t.kind == T::Eof {
                return out;
            }
            at = t.end as usize;
        }
    }

    #[test]
    fn tokens() {
        use T::*;
        assert_eq!(kinds("a.b?.c ?? d"), [Name, Dot, Name, QDot, Name, QQ, Name, Eof]);
        assert_eq!(kinds("x ? .5 : 1"), [Name, Question, Number, Colon, Number, Eof]);
        assert_eq!(kinds("a >>>= 1_000n"), [Name, GtGtGtEq, Number, Eof]);
        assert_eq!(kinds("'a\\'b' // c\n\"d\""), [Str, Str, Eof]);
        assert_eq!(kinds("x /* a\n */ y"), [Name, Name, Eof]);
        assert_eq!(kinds("#p ... => `"), [Private, Ellipsis, Arrow, Backtick, Eof]);
        assert_eq!(kinds("caf\u{e9} \u{a0}x"), [Name, Name, Eof]);
    }

    #[test]
    fn modes() {
        let l = Lexer::new(b"/a[/]\\//gi;");
        assert_eq!(l.regex(0).unwrap(), (7, 10));
        let l = Lexer::new(b"`a\\n${x}`");
        assert_eq!(l.template_piece(1).unwrap(), (Piece::Chars, 2));
        assert_eq!(l.template_piece(2).unwrap(), (Piece::Escape, 4));
        assert_eq!(l.template_piece(4).unwrap(), (Piece::Open, 6));
        let l = Lexer::new(b"a &amp; b\n  <");
        assert_eq!(l.jsx_text(0), Some(2));
        assert_eq!(l.character_reference(2), Some(7));
        assert_eq!(l.jsx_text(7), Some(12));
        let l = Lexer::new(b"\n   <");
        assert_eq!(l.jsx_text(0), None);
        let l = Lexer::new(b"data-x={1}");
        assert_eq!(l.jsx_name_end(0).unwrap(), (6, true));
    }
}
