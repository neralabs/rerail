//! The lexer: tree-sitter-rust's lexer and external scanner, read where the parser asks. The lexer
//! keeps no state: the parser asks for the next token at a byte (`Lexer::token`), and reads the few
//! places its parse state lexes otherwise itself (a field's integer after `.`, a `>>` or `&&` split
//! in two, a string's pieces).
//!
//! Every word is a `Name`, with its word code (`w::…`; 0: none) from one hash probe: tree-sitter
//! reads a keyword only where its parse state takes it (keyword extraction), so the parser decides.
//!
//! Comments are extras, in a list of their own (with their doc markers), written where the parser
//! next opens a node or reads a token.

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
/// operator is valid; the parser splits one where its state reads a shorter one (`>>` in type
/// arguments, `&&` before an operand, `||` as a closure's parameters).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum T {
    Eof,
    Name,
    /// `'a`, not a character: a lifetime or a label (or, in a token tree, `'` and a name).
    Lifetime,
    Char,
    Int,
    /// A float, as the external scanner reads one (`1.`, `1.5`, `1e5`, `2.0f32`).
    Float,
    /// A whole string, its quotes and prefix included (`"…"`, `b"…"`, `c"…"`).
    Str,
    /// A whole raw string (`r#"…"#`, `br"…"`, `cr"…"`).
    RawStr,
    /// `$name`.
    Metavariable,
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Semi,
    Comma,
    Colon,
    ColonColon,
    Dot,
    DotDot,
    DotDotDot,
    DotDotEq,
    Arrow,
    FatArrow,
    Hash,
    Dollar,
    Question,
    At,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    Bang,
    Amp,
    Pipe,
    AmpAmp,
    PipePipe,
    LtLt,
    GtGt,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    CaretEq,
    AmpEq,
    PipeEq,
    LtLtEq,
    GtGtEq,
    Eq,
    EqEq,
    Ne,
    Gt,
    Lt,
    Ge,
    Le,
    /// `'` alone (a lifetime's start where no name follows: refused by the parser).
    Quote,
}

#[derive(Clone, Copy, Debug)]
pub struct Tok {
    pub kind: T,
    /// A name's word code (`w::…`; 0: none).
    pub word: u8,
    pub start: u32,
    pub end: u32,
}

/// A comment's form: its node and its doc marker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Doc {
    /// No marker (`//`, `////…`, `/* */`, `/***/`).
    Plain,
    /// `///` or `/**`: an outer doc comment.
    Outer,
    /// `//!` or `/*!`: an inner doc comment.
    Inner,
}

/// A comment (the only extra).
#[derive(Clone, Copy, Debug)]
pub struct Extra {
    pub start: u32,
    pub end: u32,
    /// A block comment (`/* */`), not a line comment.
    pub block: bool,
    pub doc: Doc,
    /// The doc text's span (`doc_comment`): empty where it has none.
    pub doc_start: u32,
    pub doc_end: u32,
}

/// Word codes: the keywords, the primitive types, the fragment specifiers and the reserved words.
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
        AS = b"as", ASYNC = b"async", AWAIT = b"await", BREAK = b"break", CONST = b"const",
        CONTINUE = b"continue", CRATE = b"crate", DEFAULT = b"default", DYN = b"dyn", ELSE = b"else",
        ENUM = b"enum", EXTERN = b"extern", FALSE = b"false", FN = b"fn", FOR = b"for",
        GEN = b"gen", IF = b"if", IMPL = b"impl", IN = b"in", LET = b"let",
        LOOP = b"loop", MACRO_RULES = b"macro_rules", MATCH = b"match", MOD = b"mod", MOVE = b"move",
        MUT = b"mut", PUB = b"pub", RAW = b"raw", REF = b"ref", RETURN = b"return",
        SELF = b"self", STATIC = b"static", STRUCT = b"struct", SUPER = b"super", TRAIT = b"trait",
        TRUE = b"true", TRY = b"try", TYPE = b"type", UNION = b"union", UNSAFE = b"unsafe",
        USE = b"use", WHERE = b"where", WHILE = b"while", YIELD = b"yield",
        // The primitive types, in the grammar's order (`U8..=CHAR`).
        U8 = b"u8", I8 = b"i8", U16 = b"u16", I16 = b"i16", U32 = b"u32",
        I32 = b"i32", U64 = b"u64", I64 = b"i64", U128 = b"u128", I128 = b"i128",
        ISIZE = b"isize", USIZE = b"usize", F32 = b"f32", F64 = b"f64", BOOL = b"bool",
        STR = b"str", CHAR = b"char",
        // The fragment specifiers (`BLOCK..=VIS`), but those that are other words too (`literal`,
        // `lifetime` and `path` are names).
        BLOCK = b"block", EXPR = b"expr", EXPR_2021 = b"expr_2021", IDENT = b"ident", ITEM = b"item",
        LIFETIME = b"lifetime", LITERAL = b"literal", META = b"meta", PAT = b"pat", PAT_PARAM = b"pat_param",
        PATH = b"path", STMT = b"stmt", TT = b"tt", TY = b"ty", VIS = b"vis",
    }

    /// Whether `code` is a primitive type's word.
    pub const fn is_primitive(code: u8) -> bool {
        code >= U8 && code <= CHAR
    }

    /// Whether `code` is a fragment specifier's word.
    pub const fn is_fragment(code: u8) -> bool {
        code >= BLOCK && code <= VIS
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

/// White space past ASCII as the grammar's `\s` reads it (Unicode White_Space).
fn wide_space(c: char) -> bool {
    c.is_whitespace()
}

/// C's `iswalpha` for the float scanner: a letter (ASCII, or a Unicode alphabetic character).
fn is_alpha(src: &[u8], at: usize) -> bool {
    match src.get(at) {
        Some(c) if c.is_ascii() => c.is_ascii_alphabetic(),
        Some(_) => decode(src, at).is_some_and(|(c, _)| c.is_alphabetic()),
        None => false,
    }
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
                b'/' if self.at(at + 1) == b'/' => at = self.line_comment(at, extras),
                b'/' if self.at(at + 1) == b'*' => at = self.block_comment(at, extras)?,
                c if c >= 0x80 => match decode(self.src, at) {
                    Some((c, n)) if wide_space(c) => at += n,
                    _ => return Ok(at),
                },
                _ => return Ok(at),
            }
        }
    }

    /// A `//` comment from `at`, pushed: its end. `///` (not `////`) is an outer doc comment and
    /// `//!` an inner one; a doc comment's text runs to and through its line's `\n`.
    fn line_comment(&self, at: usize, extras: &mut Vec<Extra>) -> usize {
        let eol = self.blocks.eol(at + 2);
        let c = self.at(at + 2);
        let doc = if c == b'/' && self.at(at + 3) != b'/' {
            Doc::Outer
        } else if c == b'!' {
            Doc::Inner
        } else {
            Doc::Plain
        };
        // To the `\n` (a `\r` is the comment's text: `/.*/` reads it).
        let mut end = eol;
        while end < self.src.len() && self.src[end] != b'\n' {
            end = self.blocks.eol(end + 1);
        }
        if doc == Doc::Plain {
            extras.push(Extra { start: at as u32, end: end as u32, block: false, doc, doc_start: 0, doc_end: 0 });
            return end;
        }
        // A doc comment's text runs through the `\n`.
        if end < self.src.len() {
            end += 1;
        }
        extras.push(Extra { start: at as u32, end: end as u32, block: false, doc, doc_start: at as u32 + 3, doc_end: end as u32 });
        end
    }

    /// A `/*` comment from `at`, nested comments in it, pushed: its end. As the external scanner
    /// reads one: `/**` before a byte other than `*` and `/` is an outer doc comment, `/*!` an inner
    /// one; the doc text runs to the closing `*/`.
    fn block_comment(&self, at: usize, extras: &mut Vec<Extra>) -> R<usize> {
        let refuse = Err(Refusal::Lex(at as u32));
        let c = self.at(at + 2);
        let doc = if c == b'!' {
            Doc::Inner
        } else if c == b'*' && !matches!(self.at(at + 3), b'*' | b'/') {
            Doc::Outer
        } else {
            Doc::Plain
        };
        // To the closing `*/` of the outermost comment: `/*` opens one more, `*/` closes one.
        let mut depth = 1u32;
        let mut i = at + 2;
        // `/**/` and `/*!*/` close at once.
        loop {
            i = self.blocks.star(i);
            if i >= self.src.len() {
                return refuse;
            }
            match (self.src[i], self.at(i + 1)) {
                (b'*', b'/') => {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                }
                (b'/', b'*') => {
                    depth += 1;
                    i += 2;
                }
                _ => i += 1,
            }
        }
        let close = i - 2;
        if doc != Doc::Plain {
            // A nested comment, or text the scanner's states read otherwise (`/*!*/`, a `*` run at
            // the end): refused.
            if close < at + 3 || matches!(self.at(at + 3), b'*' | b'/') && close > at + 3 || self.src[at + 3..close].windows(2).any(|w| w == b"/*" || w == b"*/") {
                return refuse;
            }
        }
        let (doc_start, doc_end) = if doc == Doc::Plain { (0, 0) } else { (at as u32 + 3, close as u32) };
        extras.push(Extra { start: at as u32, end: i as u32, block: true, doc, doc_start, doc_end });
        Ok(i)
    }

    /// The next token from `at` in normal mode, white space and comments passed.
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
            b'r' if next == b'#' && self.at(s + 2) != b'"' && self.at(s + 2) != b'#' => {
                // `r#name`: a raw identifier (its word is no keyword).
                let end = self.name_end(s + 2)?;
                if end == s + 2 {
                    return refuse;
                }
                return Ok((Name, end, 0));
            }
            b'r' | b'b' | b'c' if self.raw_string_start(s).is_some() => {
                return Ok((RawStr, self.raw_string_end(s)?, 0));
            }
            b'b' | b'c' if next == b'"' => return Ok((Str, self.string_end(s + 1)?, 0)),
            b'b' if next == b'\'' => match self.char_end(s + 1) {
                Some(end) => return Ok((Char, end, 0)),
                None => return refuse,
            },
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                let end = self.name_end(s)?;
                return Ok((Name, end, word(self.src, s, end)));
            }
            b'0'..=b'9' => return self.number(s),
            b'"' => return Ok((Str, self.string_end(s)?, 0)),
            b'\'' => {
                if let Some(end) = self.char_end(s) {
                    return Ok((Char, end, 0));
                }
                let end = self.name_end(s + 1)?;
                if end == s + 1 {
                    (Quote, 1)
                } else {
                    return Ok((Lifetime, end, 0));
                }
            }
            b'$' => {
                // `$name` (`\$[a-zA-Z_]\w*`), or `$` alone.
                if next.is_ascii_alphabetic() || next == b'_' {
                    let end = self.blocks.word_end(s + 1);
                    return Ok((Metavariable, end, 0));
                }
                (Dollar, 1)
            }
            b'(' => (LParen, 1),
            b')' => (RParen, 1),
            b'[' => (LBracket, 1),
            b']' => (RBracket, 1),
            b'{' => (LBrace, 1),
            b'}' => (RBrace, 1),
            b';' => (Semi, 1),
            b',' => (Comma, 1),
            b'#' => (Hash, 1),
            b'?' => (Question, 1),
            b'@' => (At, 1),
            b':' => {
                if next == b':' {
                    (ColonColon, 2)
                } else {
                    (Colon, 1)
                }
            }
            b'.' => match (next, self.at(s + 2)) {
                (b'.', b'.') => (DotDotDot, 3),
                (b'.', b'=') => (DotDotEq, 3),
                (b'.', _) => (DotDot, 2),
                _ => (Dot, 1),
            },
            b'=' => match next {
                b'=' => (EqEq, 2),
                b'>' => (FatArrow, 2),
                _ => (Eq, 1),
            },
            b'!' => match next {
                b'=' => (Ne, 2),
                _ => (Bang, 1),
            },
            b'+' => match next {
                b'=' => (PlusEq, 2),
                _ => (Plus, 1),
            },
            b'-' => match next {
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

    /// The end of a name from `s` (`[_\p{XID_Start}][_\p{XID_Continue}]*`): ASCII word bytes a
    /// block a step, a character past ASCII by its class. `s` itself where no name starts there.
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

    /// A number at `s` (a digit): the external scanner's float where it reads one, else the
    /// grammar's `integer_literal`.
    fn number(&self, s: usize) -> R<(T, usize, u8)> {
        if let Some(end) = self.float_end(s) {
            return Ok((T::Float, end, 0));
        }
        Ok((T::Int, self.int_end(s), 0))
    }

    /// `process_float_literal` from `s`: the float's end, or `None` (an integer).
    fn float_end(&self, s: usize) -> Option<usize> {
        let at = |i: usize| self.at(i);
        let num = |c: u8| c == b'_' || c.is_ascii_digit();
        let mut i = s + 1;
        while num(at(i)) {
            i += 1;
        }
        let (mut fraction, mut exponent) = (false, false);
        if at(i) == b'.' {
            fraction = true;
            i += 1;
            if is_alpha(self.src, i) || at(i) == b'.' {
                return None;
            }
            while num(at(i)) {
                i += 1;
            }
        }
        let mut mark = i;
        if matches!(at(i), b'e' | b'E') {
            exponent = true;
            i += 1;
            if matches!(at(i), b'+' | b'-') {
                i += 1;
            }
            if !num(at(i)) {
                return Some(mark);
            }
            i += 1;
            while num(at(i)) {
                i += 1;
            }
            mark = i;
        }
        if !exponent && !fraction {
            return None;
        }
        if !matches!(at(i), b'u' | b'i' | b'f') {
            return Some(mark);
        }
        i += 1;
        if !at(i).is_ascii_digit() {
            return Some(mark);
        }
        while at(i).is_ascii_digit() {
            i += 1;
        }
        Some(i)
    }

    /// `integer_literal` from `s` (a digit): the longest of its forms, and a suffix.
    pub fn int_end(&self, s: usize) -> usize {
        let at = |i: usize| self.at(i);
        let run = |mut i: usize, ok: &dyn Fn(u8) -> bool| {
            while ok(at(i)) || at(i) == b'_' {
                i += 1;
            }
            i
        };
        let dec = run(s + 1, &|c| c.is_ascii_digit());
        let mut end = dec;
        if at(s) == b'0' {
            let other = match at(s + 1) {
                b'x' => run(s + 2, &|c| c.is_ascii_hexdigit()),
                b'b' => run(s + 2, &|c| matches!(c, b'0' | b'1')),
                b'o' => run(s + 2, &|c| matches!(c, b'0'..=b'7')),
                _ => 0,
            };
            // `0x` needs a digit or `_` after it (`[0-9a-fA-F_]+`).
            if other > s + 2 {
                end = end.max(other);
            }
        }
        // A suffix, the longest that matches.
        const SUFFIXES: [&[u8]; 14] = [b"u8", b"i8", b"u16", b"i16", b"u32", b"i32", b"u64", b"i64", b"u128", b"i128", b"isize", b"usize", b"f32", b"f64"];
        let rest = &self.src[end.min(self.src.len())..];
        let suffix = SUFFIXES.iter().filter(|x| rest.starts_with(x)).map(|x| x.len()).max().unwrap_or(0);
        end + suffix
    }

    /// A character literal from `s` (its `'`): `'` then one character or an escape, then `'`. Its
    /// end, or `None` where none is there (a lifetime).
    fn char_end(&self, s: usize) -> Option<usize> {
        let mut i = s + 1;
        match self.at(i) {
            b'\\' => {
                i = self.escape_end(i)?;
            }
            b'\'' => return Some(i + 1),
            c if c >= 0x80 => i += decode(self.src, i)?.1,
            _ if i >= self.src.len() => return None,
            _ => i += 1,
        }
        (self.at(i) == b'\'').then_some(i + 1)
    }

    /// An escape from `i` (its `\`): `\` then any character but `x` and `u`, or `u` and 4 hex
    /// digits, or `u{…}`, or `x` and 2 hex digits. Its end.
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
            c if c >= 0x80 => Some(i + 1 + decode(self.src, i + 1)?.1),
            _ if i + 1 >= self.src.len() => None,
            _ => Some(i + 2),
        }
    }

    /// A string from its `"` at `q`: its end (past the closing `"`). Escapes are read as the grammar's
    /// `escape_sequence`; one it does not read is refused.
    fn string_end(&self, q: usize) -> R<usize> {
        let mut i = q + 1;
        loop {
            i = self.blocks.string_stop(i);
            if i >= self.src.len() {
                return Err(Refusal::Lex(q as u32));
            }
            if self.src[i] == b'"' {
                return Ok(i + 1);
            }
            i = self.escape_end(i).ok_or(Refusal::Lex(i as u32))?;
        }
    }

    /// The pieces of the string whose opening `"` is at `q`: each a `string_content` run or an
    /// escape (`true`), to its closing `"`.
    pub fn string_pieces(&self, q: usize, out: &mut Vec<(u32, u32, bool)>) {
        let mut i = q + 1;
        loop {
            let stop = self.blocks.string_stop(i);
            if stop > i {
                out.push((i as u32, stop as u32, false));
            }
            if stop >= self.src.len() || self.src[stop] == b'"' {
                return;
            }
            let end = self.escape_end(stop).expect("the lexer read the string");
            out.push((stop as u32, end as u32, true));
            i = end;
        }
    }

    /// A raw string's start at `s` (`r`, `br`, `cr`, then `#`s, then `"`): the hashes' count and
    /// where its text starts.
    pub fn raw_string_start(&self, s: usize) -> Option<(usize, usize)> {
        let mut i = s;
        if matches!(self.at(i), b'b' | b'c') {
            i += 1;
        }
        if self.at(i) != b'r' {
            return None;
        }
        i += 1;
        let hashes = i;
        while self.at(i) == b'#' {
            i += 1;
        }
        (self.at(i) == b'"' && i - hashes < 256).then_some((i - hashes, i + 1))
    }

    /// A raw string's end from its start at `s`: past its closing `"` and hashes.
    fn raw_string_end(&self, s: usize) -> R<usize> {
        let (hashes, mut i) = self.raw_string_start(s).expect("a raw string");
        loop {
            i = self.blocks.string_stop(i);
            if i >= self.src.len() {
                return Err(Refusal::Lex(s as u32));
            }
            if self.src[i] == b'"' && (1..=hashes).all(|k| self.at(i + k) == b'#') {
                return Ok(i + 1 + hashes);
            }
            i += 1;
        }
    }
}
