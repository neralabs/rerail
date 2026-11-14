//! The lexer: tree-sitter-php's lexer (its `php_only` grammar), read where the parser asks. The
//! lexer keeps no state: the parser asks for the next token at a byte (`Lexer::token`), and reads
//! itself the places its parse state lexes otherwise (a string's pieces, a cast's type).
//!
//! Every word is a `Name`, with its word code (`w::…`; 0: none) from one hash probe of its bytes
//! with their case bit set: PHP's keywords are case-insensitive (`keyword()` in the grammar is a
//! case-insensitive pattern), and tree-sitter reads a keyword only where its parse state takes it
//! (keyword extraction), so the parser decides.
//!
//! A string of any form (single-quoted, double-quoted, a shell command, a heredoc, a nowdoc) is one
//! token over the whole literal; the parser reads its pieces from the bytes, as tree-sitter-php's
//! external scanner does. Comments are extras, in a list of their own, written where the parser
//! next opens a node or reads a token.

use crate::block::Blocks;

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
/// operator is valid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum T {
    Eof,
    Name,
    /// `$`.
    Dollar,
    /// `integer`.
    Int,
    /// `float`.
    Float,
    /// A single-quoted `string` (`'…'`, `b'…'`).
    Str,
    /// A double-quoted `encapsed_string` (`"…"`, `b"…"`).
    DStr,
    /// A `shell_command_expression`, `` `…` ``.
    Shell,
    /// A `heredoc` or `nowdoc`, `<<<` to its end tag.
    Heredoc,
    /// `php_tag`: `<?php`, `<?=`, `<?`.
    OpenTag,
    /// `?>`.
    CloseTag,
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Semi,
    Comma,
    Dot,
    DotEq,
    DotDotDot,
    Colon,
    ColonColon,
    Question,
    QQ,
    QQEq,
    Arrow,
    QArrow,
    FatArrow,
    Eq,
    EqEq,
    EqEqEq,
    Bang,
    Ne,
    NeEq,
    LtGt,
    Tilde,
    Plus,
    PlusPlus,
    PlusEq,
    Minus,
    MinusMinus,
    MinusEq,
    Star,
    StarStar,
    StarEq,
    StarStarEq,
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
    PipeGt,
    Lt,
    Le,
    LtLt,
    LtLtEq,
    Spaceship,
    Gt,
    Ge,
    GtGt,
    GtGtEq,
    At,
    Backslash,
    /// `#[`.
    HashLBracket,
}

#[derive(Clone, Copy, Debug)]
pub struct Tok {
    pub kind: T,
    /// A name's word code (`w::…`; 0: none).
    pub word: u8,
    pub start: u32,
    pub end: u32,
}

/// An extra: a comment.
#[derive(Clone, Copy, Debug)]
pub struct Extra {
    pub start: u32,
    pub end: u32,
}

/// Word codes, lower case: the keywords, the type words and the other words the grammar names.
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
        // The grammar's reserved keywords (`BASE_RESERVED_KEYWORD_SET`, `ABSTRACT..=YIELD`): no
        // `name` where its reserved set is `global` or `classes`.
        ABSTRACT = b"abstract", AND = b"and", AS = b"as", BREAK = b"break", CALLABLE = b"callable",
        CASE = b"case", CATCH = b"catch", CLASS = b"class", CLONE = b"clone", CONST = b"const",
        CONTINUE = b"continue", DECLARE = b"declare", DEFAULT = b"default", DO = b"do", ECHO = b"echo",
        ELSE = b"else", ELSEIF = b"elseif", ENDDECLARE = b"enddeclare", ENDFOR = b"endfor",
        ENDFOREACH = b"endforeach", ENDIF = b"endif", ENDSWITCH = b"endswitch", ENDWHILE = b"endwhile",
        EXTENDS = b"extends", FINAL = b"final", FINALLY = b"finally", FN = b"fn", FOR = b"for",
        FOREACH = b"foreach", FUNCTION = b"function", GLOBAL = b"global", GOTO = b"goto", IF = b"if",
        IMPLEMENTS = b"implements", INCLUDE = b"include", INCLUDE_ONCE = b"include_once",
        INSTANCEOF = b"instanceof", INSTEADOF = b"insteadof", INTERFACE = b"interface", MATCH = b"match",
        NAMESPACE = b"namespace", NEW = b"new", OR = b"or", PRINT = b"print", PRIVATE = b"private",
        PROTECTED = b"protected", PUBLIC = b"public", READONLY = b"readonly", REQUIRE = b"require",
        REQUIRE_ONCE = b"require_once", RETURN = b"return", STATIC = b"static", SWITCH = b"switch",
        THROW = b"throw", TRAIT = b"trait", TRY = b"try", USE = b"use", VAR = b"var", WHILE = b"while",
        XOR = b"xor", YIELD = b"yield",
        // The other reserved words (`OTHER_RESERVED_KEYWORD_SET`, `BOOL..=VOID`): no class's name.
        BOOL = b"bool", FALSE = b"false", FLOAT = b"float", INT = b"int", ITERABLE = b"iterable",
        MIXED = b"mixed", NEVER = b"never", NULL = b"null", OBJECT = b"object", STRING = b"string",
        TRUE = b"true", VOID = b"void",
        // Words the grammar names that are names elsewhere.
        ARRAY = b"array", LIST = b"list", SELF = b"self", PARENT = b"parent", EXIT = b"exit",
        UNSET = b"unset", ENUM = b"enum", FROM = b"from", TICKS = b"ticks", ENCODING = b"encoding",
        STRICT_TYPES = b"strict_types", BINARY = b"binary", BOOLEAN = b"boolean", DOUBLE = b"double",
        INTEGER = b"integer", REAL = b"real",
    }

    /// Whether `code` is in the grammar's `global` reserved set (never a `name` there).
    pub const fn is_reserved(code: u8) -> bool {
        code >= ABSTRACT && code <= YIELD
    }

    /// Whether `code` is in the grammar's `classes` reserved set.
    pub const fn is_class_reserved(code: u8) -> bool {
        code >= ABSTRACT && code <= VOID
    }

    /// Whether `code` is a cast's type word (`cast_type`).
    pub const fn is_cast_type(code: u8) -> bool {
        matches!(code, ARRAY | BINARY | BOOL | BOOLEAN | DOUBLE | FLOAT | INT | INTEGER | OBJECT | REAL | STRING | UNSET)
    }
}

/// A word's first 8 bytes as one integer, little-endian, their case bit set, zeros past its end.
const fn word_key(w: &[u8]) -> u64 {
    let mut k = 0u64;
    let mut i = 0;
    while i < w.len() && i < 8 {
        k |= ((w[i] | 0x20) as u64) << (8 * i);
        i += 1;
    }
    k
}

/// A word's slot in `SLOTS`, by its first 8 bytes and its length: a multiplier under which every
/// word takes a slot of its own of 4,096 (a perfect hash; `SLOTS` is refused at compile time where
/// two share one).
#[inline(always)]
const fn word_slot(key: u64, len: usize) -> usize {
    ((key ^ (len as u64).wrapping_mul(0x9e37_79b9)).wrapping_mul(HASH) >> 52) as usize
}

const SLOT_COUNT: usize = 4096;

#[allow(long_running_const_eval)]
const HASH: u64 = find_hash();

/// The first multiplier (of a fixed sequence) under which the words' slots are distinct.
const fn find_hash() -> u64 {
    let mut m = 0x8f58_640b_360e_7c81u64;
    loop {
        let mut used = [false; SLOT_COUNT];
        let mut ok = true;
        let mut i = 1;
        while i < w::TEXT.len() {
            let t = w::TEXT[i];
            let s = ((word_key(t) ^ (t.len() as u64).wrapping_mul(0x9e37_79b9)).wrapping_mul(m) >> 52) as usize;
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

/// Each slot's word code (0: none): 4 KB, in the first cache levels.
static SLOTS: [u8; SLOT_COUNT] = {
    let mut t = [0u8; SLOT_COUNT];
    let mut i = 1;
    while i < w::TEXT.len() {
        let text = w::TEXT[i];
        assert!(text.len() <= 16);
        let slot = word_slot(word_key(text), text.len());
        assert!(t[slot] == 0, "two words in a slot");
        t[slot] = i as u8;
        i += 1;
    }
    t
};

/// Each word's first 8 bytes and its length, by code.
static KEYS: [(u64, u8); 256] = {
    let mut t = [(0u64, 0u8); 256];
    let mut i = 1;
    while i < w::TEXT.len() {
        t[i] = (word_key(w::TEXT[i]), w::TEXT[i].len() as u8);
        i += 1;
    }
    t
};

/// The word `src[i..end]`'s code, in any case: its first 8 bytes loaded as one integer, their case
/// bit set, masked to its length, hashed to its one slot and compared once (and its tail, for the
/// few words past 8 bytes). A name's bytes are `[A-Za-z0-9_]` and bytes from 0x80: setting the case
/// bit maps no other name byte onto a keyword's.
#[inline(always)]
pub fn word(src: &[u8], i: usize, end: usize) -> u8 {
    let len = end - i;
    if len.wrapping_sub(1) >= 16 {
        return 0;
    }
    let raw = match src.get(i..i + 8) {
        Some(b) => u64::from_le_bytes(b.try_into().expect("8 bytes")) | 0x2020_2020_2020_2020,
        None => word_key(&src[i..end]),
    };
    let k = if len >= 8 { raw } else { raw & (u64::MAX >> (64 - 8 * len)) };
    let code = SLOTS[word_slot(k, len)];
    let (wk, wl) = KEYS[usize::from(code)];
    if code == 0 || wk != k || usize::from(wl) != len {
        return 0;
    }
    if len > 8 && !w::TEXT[usize::from(code)][8..].eq_ignore_ascii_case(&src[i + 8..end]) {
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

/// Whether `c` (past ASCII) is in `name`'s ranges: `\u0080-\u009f¡- ‌- `
/// `⁡-﻾＀-￿`.
fn is_name_char(c: char) -> bool {
    matches!(c as u32, 0x80..=0x9f | 0xa1..=0x200a | 0x200c..=0x205f | 0x2061..=0xfefe | 0xff00..=0xffff)
}

/// Whether `c` (past ASCII) is white space: `[\s ​⁠﻿]` (JavaScript's `\s`).
fn is_space(c: char) -> bool {
    matches!(c as u32, 0xa0 | 0x1680 | 0x2000..=0x200b | 0x2028 | 0x2029 | 0x202f | 0x205f | 0x2060 | 0x3000 | 0xfeff)
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

    /// From `at`: white space and comments passed (pushed to `extras`); where the next token starts.
    #[inline]
    pub fn skip(&self, mut at: usize, extras: &mut Vec<Extra>) -> R<usize> {
        loop {
            at = self.blocks.blank_end(at);
            match self.at(at) {
                b'\n' | b'\r' => at += 1,
                b'/' if self.at(at + 1) == b'/' => {
                    let end = self.line_comment_end(at + 2);
                    extras.push(Extra { start: at as u32, end: end as u32 });
                    at = end;
                }
                b'#' if self.at(at + 1) != b'[' && at < self.src.len() => {
                    // `#` alone before `?`, a line end or the end; else `#` and a line comment's body.
                    let end = if matches!(self.at(at + 1), b'?' | b'\r' | b'\n') || at + 1 >= self.src.len() { at + 1 } else { self.line_comment_end(at + 2) };
                    extras.push(Extra { start: at as u32, end: end as u32 });
                    at = end;
                }
                b'/' if self.at(at + 1) == b'*' => {
                    let mut i = at + 2;
                    loop {
                        i = self.blocks.star(i);
                        if i >= self.src.len() {
                            return Err(Refusal::Lex(at as u32));
                        }
                        if self.src[i] == b'*' && self.at(i + 1) == b'/' && i >= at + 2 {
                            i += 2;
                            break;
                        }
                        i += 1;
                    }
                    extras.push(Extra { start: at as u32, end: i as u32 });
                    at = i;
                }
                c if c >= 0x80 => match decode(self.src, at) {
                    Some((c, n)) if is_space(c) => at += n,
                    _ => return Ok(at),
                },
                _ => return Ok(at),
            }
        }
    }

    /// A line comment's body from `i`: `([^?\r\n]|\?[^>\r\n])*(\?\r?\n)?`. Its end.
    fn line_comment_end(&self, mut i: usize) -> usize {
        loop {
            i = self.blocks.line_stop(i);
            if i >= self.src.len() || self.src[i] != b'?' {
                return i;
            }
            match self.at(i + 1) {
                b'\n' => return i + 2,
                b'\r' if self.at(i + 2) == b'\n' => return i + 3,
                b'>' | b'\r' => return i,
                _ if i + 1 >= self.src.len() => return i,
                _ => i += 1,
            }
        }
    }

    /// The next token from `at`, white space and extras passed.
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
            b'b' | b'B' if next == b'\'' => {
                return match self.single_end(s + 1) {
                    Some(end) => Ok((Str, end, 0)),
                    None => refuse,
                };
            }
            b'b' | b'B' if next == b'"' => {
                return match self.double_end(s + 1, b'"') {
                    Some(end) => Ok((DStr, end, 0)),
                    None => refuse,
                };
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                let end = self.name_end(s)?;
                return Ok((Name, end, word(self.src, s, end)));
            }
            b'0'..=b'9' => return self.number(s),
            b'.' if next.is_ascii_digit() => return self.number(s),
            b'$' => (Dollar, 1),
            b'\'' => {
                return match self.single_end(s) {
                    Some(end) => Ok((Str, end, 0)),
                    None => refuse,
                };
            }
            b'"' => {
                return match self.double_end(s, b'"') {
                    Some(end) => Ok((DStr, end, 0)),
                    None => refuse,
                };
            }
            b'`' => {
                return match self.double_end(s, b'`') {
                    Some(end) => Ok((Shell, end, 0)),
                    None => refuse,
                };
            }
            b'#' if next == b'[' => (HashLBracket, 2),
            b'(' => (LParen, 1),
            b')' => (RParen, 1),
            b'[' => (LBracket, 1),
            b']' => (RBracket, 1),
            b'{' => (LBrace, 1),
            b'}' => (RBrace, 1),
            b';' => (Semi, 1),
            b',' => (Comma, 1),
            b'~' => (Tilde, 1),
            b'@' => (At, 1),
            b'\\' => (Backslash, 1),
            b'?' => match (next, self.at(s + 2)) {
                (b'>', _) => (CloseTag, 2),
                (b'-', b'>') => (QArrow, 3),
                (b'?', b'=') => (QQEq, 3),
                (b'?', _) => (QQ, 2),
                _ => (Question, 1),
            },
            b':' => {
                if next == b':' {
                    (ColonColon, 2)
                } else {
                    (Colon, 1)
                }
            }
            b'.' => match (next, self.at(s + 2)) {
                (b'.', b'.') => (DotDotDot, 3),
                (b'=', _) => (DotEq, 2),
                _ => (Dot, 1),
            },
            b'=' => match (next, self.at(s + 2)) {
                (b'=', b'=') => (EqEqEq, 3),
                (b'=', _) => (EqEq, 2),
                (b'>', _) => (FatArrow, 2),
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
                b'-' => (MinusMinus, 2),
                b'=' => (MinusEq, 2),
                b'>' => (Arrow, 2),
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
                b'>' => (PipeGt, 2),
                _ => (Pipe, 1),
            },
            b'<' => match (next, self.at(s + 2)) {
                (b'<', b'<') => {
                    return match self.heredoc_end(s) {
                        Some(end) => Ok((Heredoc, end, 0)),
                        None => refuse,
                    };
                }
                (b'?', _) => {
                    let t = &self.src[s + 2..(s + 5).min(self.src.len())];
                    let n = if t.len() == 3 && t.eq_ignore_ascii_case(b"php") {
                        5
                    } else if t.first() == Some(&b'=') {
                        3
                    } else {
                        2
                    };
                    (OpenTag, n)
                }
                (b'<', b'=') => (LtLtEq, 3),
                (b'<', _) => (LtLt, 2),
                (b'=', b'>') => (Spaceship, 3),
                (b'=', _) => (Le, 2),
                (b'>', _) => (LtGt, 2),
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

    /// The end of a name from `s` (`[_a-zA-Z…][_a-zA-Z…\d]*`, `…` the grammar's ranges past ASCII):
    /// ASCII word bytes a block a step, a character past ASCII by its range. `s` itself where no
    /// name starts there.
    pub fn name_end(&self, s: usize) -> R<usize> {
        let c = self.at(s);
        if c.is_ascii_digit() || (c < 0x80 && !crate::block::is_word(c)) {
            return Ok(s);
        }
        let mut i = s;
        loop {
            i = self.blocks.word_end(i);
            if i >= self.src.len() || self.src[i] < 0x80 {
                return Ok(i);
            }
            let Some((ch, n)) = decode(self.src, i) else { return Err(Refusal::Lex(i as u32)) };
            if !is_name_char(ch) {
                return Ok(i);
            }
            i += n;
        }
    }

    /// `\d+(_\d+)*`-shaped digits from `i` by `digit`: the end of the longest run, `i` where none.
    fn run(&self, i: usize, digit: impl Fn(u8) -> bool) -> usize {
        let mut k = i;
        while digit(self.at(k)) {
            k += 1;
        }
        if k == i {
            return i;
        }
        while self.at(k) == b'_' && digit(self.at(k + 1)) {
            k += 1;
            while digit(self.at(k)) {
                k += 1;
            }
        }
        k
    }

    /// A number at `s` (a digit, or a `.` before one): the longest of `integer` and `float`.
    fn number(&self, s: usize) -> R<(T, usize, u8)> {
        let dec = |c: u8| c.is_ascii_digit();
        let (c0, c1) = (self.at(s), self.at(s + 1));
        let mut int_end = s;
        if c0 == b'0' && matches!(c1, b'x' | b'X') {
            int_end = self.run(s + 2, |c| c.is_ascii_hexdigit());
            if int_end == s + 2 {
                int_end = s + 1;
            }
        } else if c0 == b'0' && matches!(c1, b'b' | b'B') {
            int_end = self.run(s + 2, |c| c == b'0' || c == b'1');
            if int_end == s + 2 {
                int_end = s + 1;
            }
        } else if c0 == b'0' {
            // `0[oO]?[0-7]*(_[0-7]+)*`.
            let o = if matches!(c1, b'o' | b'O') { s + 2 } else { s + 1 };
            let e = self.run(o, |c| (b'0'..=b'7').contains(&c));
            int_end = if e == o && o == s + 2 { s + 2 } else { e.max(s + 1) };
        } else if c0.is_ascii_digit() {
            int_end = self.run(s, dec);
        }
        // `float`: `\d*(_\d+)*((\.\d*(_\d+)*)?([eE][\+-]?\d+(_\d+)*)|(\.\d*(_\d+)*)([eE][\+-]?\d+(_\d+)*)?)`.
        let int = if c0.is_ascii_digit() { self.run(s, dec) } else { s };
        let exponent = |i: usize| -> Option<usize> {
            if !matches!(self.at(i), b'e' | b'E') {
                return None;
            }
            let k = if matches!(self.at(i + 1), b'+' | b'-') { i + 2 } else { i + 1 };
            let e = self.run(k, dec);
            (e > k).then_some(e)
        };
        let mut float_end = s;
        if self.at(int) == b'.' {
            let f = if self.at(int + 1).is_ascii_digit() { self.run(int + 1, dec) } else { int + 1 };
            float_end = exponent(f).unwrap_or(f);
        } else if let Some(e) = exponent(int) {
            float_end = e;
        }
        if float_end > int_end {
            return Ok((T::Float, float_end, 0));
        }
        if int_end == s {
            return Err(Refusal::Lex(s as u32));
        }
        Ok((T::Int, int_end, 0))
    }

    /// A single-quoted string from its `'` at `s`: past its closer. `None` where the file ends first.
    pub fn single_end(&self, s: usize) -> Option<usize> {
        let mut i = s + 1;
        loop {
            i = self.blocks.quote_stop(i);
            match self.src.get(i)? {
                b'\'' => return Some(i + 1),
                _ => i += 2,
            }
        }
    }

    /// A double-quoted string (or a shell command, `close` its quote) from its quote at `s`: past
    /// its closer, its interpolations (`{$…}`, `${…}`) passed as balanced braces.
    pub fn double_end(&self, s: usize, close: u8) -> Option<usize> {
        let mut i = s + 1;
        loop {
            i = self.blocks.string_stop(i);
            let c = *self.src.get(i)?;
            match c {
                _ if c == close => return Some(i + 1),
                b'\\' => i += 2,
                b'{' if self.at(i + 1) == b'$' => i = self.braces_end(i + 1)?,
                b'$' if self.at(i + 1) == b'{' => i = self.braces_end(i + 2)?,
                _ => i += 1,
            }
        }
    }

    /// From inside an interpolation's `{` (at `i`, past it): past its closing `}`, nested braces,
    /// strings and comments passed.
    fn braces_end(&self, mut i: usize) -> Option<usize> {
        let mut depth = 0u32;
        loop {
            match *self.src.get(i)? {
                b'{' => depth += 1,
                b'}' if depth == 0 => return Some(i + 1),
                b'}' => depth -= 1,
                b'\'' => {
                    i = self.single_end(i)?;
                    continue;
                }
                b'"' => {
                    i = self.double_end(i, b'"')?;
                    continue;
                }
                b'`' => {
                    i = self.double_end(i, b'`')?;
                    continue;
                }
                b'<' if self.src[i..].starts_with(b"<<<") => return None,
                b'/' if matches!(self.at(i + 1), b'/' | b'*') => return None,
                b'#' => return None,
                _ => {}
            }
            i += 1;
        }
    }

    /// A heredoc's or nowdoc's opening from `s` (its `<<<`): whether a nowdoc, its word's span, and
    /// where its body's line starts (past the opening line's end). `None` where none opens.
    pub fn heredoc_open(&self, s: usize) -> Option<(bool, usize, usize, usize)> {
        let mut i = s + 3;
        while matches!(self.at(i), b' ' | b'\t') {
            i += 1;
        }
        let quote = self.at(i);
        let nowdoc = quote == b'\'';
        if matches!(quote, b'\'' | b'"') {
            i += 1;
        }
        let ws = i;
        let we = self.blocks.word_end(i);
        if we == ws || self.at(ws).is_ascii_digit() || self.at(we) >= 0x80 {
            return None;
        }
        i = we;
        if matches!(quote, b'\'' | b'"') {
            if self.at(i) != quote {
                return None;
            }
            i += 1;
        }
        // The opening line ends here (`\r?\n|\r`).
        match self.at(i) {
            b'\r' if self.at(i + 1) == b'\n' => i += 2,
            b'\r' | b'\n' => i += 1,
            _ => return None,
        }
        Some((nowdoc, ws, we, i))
    }

    /// Where a heredoc's end tag `word` stands on the line from `line` (white space before it, then
    /// the word, then white space or `;`, `,`, `)`, a line end): the tag's span.
    pub fn end_tag(&self, line: usize, word: &[u8]) -> Option<(usize, usize)> {
        let mut i = line;
        while matches!(self.at(i), b' ' | b'\t' | 0x0b | 0x0c) {
            i += 1;
        }
        if !self.src[i..].starts_with(word) {
            return None;
        }
        let e = i + word.len();
        let mut k = e;
        if !matches!(self.at(k), b' ' | b'\t' | 0x0b | 0x0c | b';' | b',' | b')' | b'\n' | b'\r') {
            return None;
        }
        while matches!(self.at(k), b' ' | b'\t' | 0x0b | 0x0c) {
            k += 1;
        }
        matches!(self.at(k), b';' | b',' | b')' | b'\n' | b'\r').then_some((i, e))
    }

    /// A heredoc or nowdoc from `s` (its `<<<`): past its end tag. The tag is looked for at each
    /// line's start.
    fn heredoc_end(&self, s: usize) -> Option<usize> {
        let (_, ws, we, mut line) = self.heredoc_open(s)?;
        let word = &self.src[ws..we];
        loop {
            if line >= self.src.len() {
                return None;
            }
            if let Some((_, e)) = self.end_tag(line, word) {
                return Some(e);
            }
            let e = self.blocks.eol(line);
            if e >= self.src.len() {
                return None;
            }
            line = if self.src[e] == b'\r' && self.at(e + 1) == b'\n' { e + 2 } else { e + 1 };
        }
    }
}
