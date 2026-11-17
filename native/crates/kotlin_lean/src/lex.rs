//! The lexer: tree-sitter-kotlin's lexer and its external scanner, read where the parser asks. The
//! lexer keeps no state: the parser asks for the next token at a byte (`Lexer::token`), and reads
//! itself the places its parse state lexes otherwise (a `?.` split in a type, a string's pieces).
//!
//! Every word is a `Name`, with its word code (`w::…`; 0: none) from one hash probe: tree-sitter
//! reads a keyword only where its parse state takes it (keyword extraction), so the parser decides.
//!
//! The external scanner's automatic semicolon is read from the bytes between two tokens
//! (`Lexer::asi`), where the parser's state takes one. A string is one token over the whole literal
//! (its interpolations' brackets and strings passed); the parser reads its pieces from the bytes.
//!
//! Comments are extras, in a list of their own, written where the parser next opens a node or
//! reads a token.

use crate::block::{self, Blocks, Lexed};
use rerail_simd8::swar::{self, Class};

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
/// operator is valid; the parser splits one where its state reads a shorter one (`?.` after a type).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum T {
    Eof,
    /// A name: `[\p{L}_][\p{L}_\p{Nd}]*`, or a backtick name (word code `w::NONE`, `Tok::tick`).
    Name,
    /// `integer_literal`.
    Int,
    Hex,
    Bin,
    Real,
    /// A character literal, `'` to `'`.
    Char,
    /// A string literal: its `$`s, `"` or `"""`, to its end (its pieces are read by the parser).
    Str,
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Semi,
    Comma,
    Dot,
    DotDot,
    DotDotLt,
    QuestionDot,
    Elvis,
    Question,
    Colon,
    ColonColon,
    Arrow,
    At,
    Eq,
    EqEq,
    EqEqEq,
    Bang,
    BangBang,
    Ne,
    NeEq,
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
    Lt,
    Le,
    Gt,
    Ge,
    AmpAmp,
    Amp,
    PipePipe,
    /// `#!` at the file's start.
    Shebang,
}

#[derive(Clone, Copy, Debug)]
pub struct Tok {
    pub kind: T,
    /// A name's word code (`w::…`; 0: none).
    pub word: u8,
    /// A line end in the white space before the token (outside comments).
    pub nl: bool,
    /// A backtick name, or a name past ASCII (never a label).
    pub tick: bool,
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

/// Word codes: the keywords and the soft keywords.
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
        // Hard keywords: never a `simple_identifier`.
        AS = b"as", BREAK = b"break", CLASS = b"class", CONTINUE = b"continue", DO = b"do", ELSE = b"else",
        FALSE = b"false", FOR = b"for", FUN = b"fun", IF = b"if", IN = b"in", INTERFACE = b"interface",
        IS = b"is", NULL = b"null", OBJECT = b"object", PACKAGE = b"package", RETURN = b"return",
        SUPER = b"super", THIS = b"this", THROW = b"throw", TRUE = b"true", TRY = b"try",
        TYPEALIAS = b"typealias", VAL = b"val", VAR = b"var", WHEN = b"when", WHILE = b"while",
        IMPORT = b"import", BY = b"by", OUT = b"out", FILE = b"file", L = b"L",
        // Soft keywords: also a `simple_identifier` (`SOFT..`).
        EXPECT = b"expect", DATA = b"data", INNER = b"inner", VALUE = b"value", ACTUAL = b"actual",
        SET = b"set", GET = b"get", OVERRIDE = b"override", SUSPEND = b"suspend", ANNOTATION = b"annotation",
        SEALED = b"sealed", LATEINIT = b"lateinit", TAILREC = b"tailrec", OPERATOR = b"operator",
        INFIX = b"infix", INLINE = b"inline", EXTERNAL = b"external", PUBLIC = b"public",
        PRIVATE = b"private", INTERNAL = b"internal", PROTECTED = b"protected", ABSTRACT = b"abstract",
        FINAL = b"final", OPEN = b"open", CONST = b"const", VARARG = b"vararg", NOINLINE = b"noinline",
        CROSSINLINE = b"crossinline", REIFIED = b"reified", FIELD = b"field", PROPERTY = b"property",
        RECEIVER = b"receiver", PARAM = b"param", SETPARAM = b"setparam", DELEGATE = b"delegate",
        COMPANION = b"companion", CONSTRUCTOR = b"constructor", INIT = b"init", DYNAMIC = b"dynamic",
        WHERE = b"where", CATCH = b"catch", FINALLY = b"finally", ENUM = b"enum",
    }

    /// The first soft keyword: a code from here on is also a `simple_identifier`.
    pub const SOFT: u8 = EXPECT;
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
pub fn decode(src: &[u8], at: usize) -> Option<(char, usize)> {
    let s = std::str::from_utf8(&src[at..(at + 4).min(src.len())]).or_else(|e| std::str::from_utf8(&src[at..at + e.valid_up_to()])).ok()?;
    let c = s.chars().next()?;
    Some((c, c.len_utf8()))
}

/// Whether `c` (past ASCII) is a letter (`\p{L}`) the lexer reads for certain: the scripts where
/// Unicode's `Alphabetic` is `\p{L}`. Any other character past ASCII in a name is refused.
fn letter(c: char) -> Option<bool> {
    let u = c as u32;
    let sure = matches!(u, 0xc0..=0x24f | 0x370..=0x3ff | 0x400..=0x4ff | 0x3040..=0x30ff | 0x4e00..=0x9fff | 0xac00..=0xd7a3)
        && u != 0xd7
        && u != 0xf7
        && u != 0x37e
        && u != 0x387;
    if sure {
        Some(c.is_alphabetic())
    } else if c.is_whitespace() || matches!(u, 0x2000..=0x2bff | 0x3000..=0x303f | 0xfe00..=0xfe0f | 0xff00..=0xff0f) {
        // Spaces, punctuation and symbols: no letter.
        Some(false)
    } else {
        None
    }
}

/// Digit classes (`swar::class_end`, 8 bytes a step) and a quoted name's body (any byte but
/// `` ` `` and a line end).
static DEC: Class = swar::class_of(b"0123456789");
static HEX: Class = swar::class_of(b"0123456789abcdefABCDEF");
static BIN: Class = swar::class_of(b"01");
static TICK_BODY: Class = {
    let mut t = [true; 256];
    t[b'`' as usize] = false;
    t[b'\n' as usize] = false;
    t[b'\r' as usize] = false;
    t
};

/// Bytes that start a name (`[A-Za-z_]`; one past ASCII goes the long way).
static NAME_START: [bool; 256] = {
    let mut t = [false; 256];
    let mut c = 0;
    while c < 256 {
        t[c] = (c as u8).is_ascii_alphabetic() || c == b'_' as usize;
        c += 1;
    }
    t
};

// The bytes after a one-byte token that make it longer (`FOLLOW`'s bits, a token's `ONE`).
const F_DOT: u16 = 1;
const F_COLON: u16 = 2;
const F_EQ: u16 = 4;
const F_BANG: u16 = 8;
const F_PLUS: u16 = 16;
const F_MINUS: u16 = 32;
const F_GT: u16 = 64;
const F_AMP: u16 = 128;
const F_DIGIT: u16 = 256;

static FOLLOW: [u16; 256] = {
    let mut t = [0u16; 256];
    t[b'.' as usize] = F_DOT;
    t[b':' as usize] = F_COLON;
    t[b'=' as usize] = F_EQ;
    t[b'!' as usize] = F_BANG;
    t[b'+' as usize] = F_PLUS;
    t[b'-' as usize] = F_MINUS;
    t[b'>' as usize] = F_GT;
    t[b'&' as usize] = F_AMP;
    let mut d = b'0';
    while d <= b'9' {
        t[d as usize] = F_DIGIT;
        d += 1;
    }
    t
};

/// A one-byte token by its byte (`token_at`'s): its kind, and the next bytes that make it another
/// (`u16::MAX`: no one-byte token, or one read the long way).
static ONE: [(T, u16); 256] = {
    let mut t = [(T::Eof, u16::MAX); 256];
    t[b'(' as usize] = (T::LParen, 0);
    t[b')' as usize] = (T::RParen, 0);
    t[b'[' as usize] = (T::LBracket, 0);
    t[b']' as usize] = (T::RBracket, 0);
    t[b'{' as usize] = (T::LBrace, 0);
    t[b'}' as usize] = (T::RBrace, 0);
    t[b';' as usize] = (T::Semi, 0);
    t[b',' as usize] = (T::Comma, 0);
    t[b'@' as usize] = (T::At, 0);
    t[b'?' as usize] = (T::Question, F_DOT | F_COLON);
    t[b':' as usize] = (T::Colon, F_COLON);
    t[b'.' as usize] = (T::Dot, F_DOT | F_DIGIT);
    t[b'=' as usize] = (T::Eq, F_EQ);
    t[b'!' as usize] = (T::Bang, F_EQ | F_BANG);
    t[b'+' as usize] = (T::Plus, F_PLUS | F_EQ);
    t[b'-' as usize] = (T::Minus, F_MINUS | F_EQ | F_GT);
    t[b'*' as usize] = (T::Star, F_EQ);
    t[b'%' as usize] = (T::Percent, F_EQ);
    t[b'&' as usize] = (T::Amp, F_AMP);
    t[b'<' as usize] = (T::Lt, F_EQ);
    t[b'>' as usize] = (T::Gt, F_EQ);
    t
};

/// An automatic semicolon (the external scanner's `_automatic_semicolon`) where the parser's state
/// takes one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Asi {
    /// None: the token after continues the statement.
    No,
    /// A semicolon of no width (a line end, the file's end).
    Zero,
    /// A `;`, at this byte: the semicolon is the `;` read.
    Semi(u32),
    /// `constructor` after a line end where a primary constructor may start: its keyword.
    Ctor,
    /// A semicolon of no width at this byte, past a block comment on the line before it: the
    /// comment is in no node (the scanner reads it and marks the token's end past it, before a word
    /// or a `!` on the comment's next line).
    Swallow(u32),
}

/// Where the scanner's state reads `by` (`_by_delegation_hint` valid) or a primary constructor
/// (`_primary_constructor_keyword` valid).
#[derive(Clone, Copy, Debug, Default)]
pub struct AsiCtx {
    pub by: bool,
    pub ctor: bool,
}

#[inline(always)]
fn space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

#[inline(always)]
fn wordc(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
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
    /// token starts, and whether a line end is in the white space passed.
    #[inline]
    pub fn skip(&self, mut at: usize, extras: &mut Vec<(u32, u32, u32)>) -> R<(usize, bool)> {
        // A token touching the last one (`f(`, `.b`): no white space, no comment.
        if let Some(&c) = self.src.get(at) {
            if c < 0x80 && c != b'/' && !block::is_space(c) {
                return Ok((at, false));
            }
        }
        let mut nl = false;
        loop {
            loop {
                at = self.blocks.blank_end(at);
                match self.at(at) {
                    b'\n' | b'\r' => {
                        nl = true;
                        at += 1
                    }
                    _ => break,
                }
            }
            if self.at(at) >= 0x80 {
                // White space past ASCII (`\s` reads it): refused.
                if let Some((c, _)) = decode(self.src, at) {
                    if c.is_whitespace() {
                        return Err(Refusal::Lex(at as u32));
                    }
                }
            }
            let Some(x) = self.comment(at)? else { return Ok((at, nl)) };
            let kind = if x.block { crate::K::MultilineComment } else { crate::K::LineComment };
            extras.push((crate::tw::key(kind, 0, true), x.start, x.end));
            at = x.end as usize;
        }
    }

    /// The comment starting at `at`, if one does; an unclosed `/*` refused. Block comments nest.
    #[inline]
    pub fn comment(&self, at: usize) -> R<Option<Extra>> {
        if self.at(at) != b'/' {
            return Ok(None);
        }
        match self.at(at + 1) {
            b'/' => {
                // To the `\n` (a `\r` is the comment's text: `.*` reads it).
                let mut end = self.blocks.eol(at + 2);
                while end < self.src.len() && self.src[end] != b'\n' {
                    end = self.blocks.eol(end + 1);
                }
                Ok(Some(Extra { start: at as u32, end: end as u32, block: false }))
            }
            b'*' => match self.block_comment_end(at) {
                Some(end) => Ok(Some(Extra { start: at as u32, end: end as u32, block: true })),
                None => Err(Refusal::Lex(at as u32)),
            },
            _ => Ok(None),
        }
    }

    /// The end of the (nested) block comment at `at` (its `/*`); `None` where the file ends first.
    pub fn block_comment_end(&self, at: usize) -> Option<usize> {
        let mut i = at + 2;
        let mut depth = 1u32;
        let mut after_star = false;
        loop {
            // The scanner's states: after a `*`, a `/` closes; after a `/`, a `*` opens.
            let j = self.blocks.star(i);
            if j >= self.src.len() {
                return None;
            }
            if j > i {
                after_star = false;
            }
            i = j;
            if self.src[i] == b'*' {
                after_star = true;
                i += 1;
            } else {
                i += 1;
                if after_star {
                    after_star = false;
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                } else if self.at(i) == b'*' {
                    depth += 1;
                    i += 1;
                }
            }
        }
    }

    /// Tokens from `at` on into `out`, a block at a time (`Blocks::lex_blocks`): names and
    /// one-byte punctuation straight from the block's masks and two tables, every other token by
    /// `token`. Returns where the next is to be read from; at least `min` tokens are made unless
    /// the text ends.
    pub fn lex_run(&self, at: usize, out: &mut Vec<Tok>, min: usize, extras: &mut Vec<(u32, u32, u32)>) -> R<usize> {
        let src = self.src;
        self.blocks.lex_blocks(
            at,
            out,
            min,
            extras,
            #[inline(always)]
            |extras, pos, run, nl| {
                let c = src[pos];
                let tok = |kind, word, tick, e: usize| Ok(Lexed::Tok(Tok { kind, word, nl, tick, start: pos as u32, end: e as u32 }, e));
                if NAME_START[c as usize] {
                    // A name of ASCII bytes (past the block too), ended before ASCII.
                    let e = if run == usize::MAX { self.blocks.word_end(pos) } else { pos + run };
                    if self.at(e) < 0x80 {
                        return tok(T::Name, word(src, pos, e), false, e);
                    }
                } else {
                    let (kind, bad) = ONE[c as usize];
                    if bad != u16::MAX && FOLLOW[self.at(pos + 1) as usize] & bad == 0 {
                        return tok(kind, 0, false, pos + 1);
                    }
                    if c >= 0x80 {
                        // White space past ASCII is refused (`skip`): the long way.
                        return Ok(Lexed::Slow);
                    }
                    if c == b'/' {
                        if let Some(x) = self.comment(pos)? {
                            let kind = if x.block { crate::K::MultilineComment } else { crate::K::LineComment };
                            extras.push((crate::tw::key(kind, 0, true), x.start, x.end));
                            return Ok(Lexed::Extra(x.end as usize));
                        }
                    }
                }
                // Any other token from its first byte, as `token` reads it there.
                let (kind, e, word, tick) = self.token_at(pos)?;
                tok(kind, word, tick, e)
            },
            |extras, gap, nl| {
                let mut t = self.token(gap, extras)?;
                t.nl |= nl;
                Ok((t, t.end as usize, t.kind == T::Eof))
            },
        )
    }

    /// The next token from `at`, white space and comments passed.
    #[inline]
    pub fn token(&self, at: usize, extras: &mut Vec<(u32, u32, u32)>) -> R<Tok> {
        let (s, nl) = self.skip(at, extras)?;
        let (kind, end, word, tick) = self.token_at(s)?;
        Ok(Tok { kind, word, nl, tick, start: s as u32, end: end as u32 })
    }

    /// The token at `s` (no white space before it): its kind, end, word code and tick flag.
    fn token_at(&self, s: usize) -> R<(T, usize, u8, bool)> {
        use T::*;
        if s >= self.src.len() {
            return Ok((Eof, s, 0, false));
        }
        let c = self.src[s];
        let refuse = Err(Refusal::Lex(s as u32));
        let next = self.at(s + 1);
        let (kind, len) = match c {
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                let (end, tick) = self.name_end(s)?;
                let code = if tick { 0 } else { word(self.src, s, end) };
                return Ok((Name, end, code, tick));
            }
            b'`' => {
                let i = swar::class_end(self.src, s + 1, &TICK_BODY);
                if i == s + 1 || self.at(i) != b'`' {
                    return refuse;
                }
                return Ok((Name, i + 1, 0, true));
            }
            b'0'..=b'9' => return self.number(s),
            b'.' if next.is_ascii_digit() => return self.number(s),
            b'"' => {
                return match self.string_end(s) {
                    Some(end) => Ok((Str, end, 0, false)),
                    None => refuse,
                };
            }
            b'$' => {
                let mut i = s;
                while self.at(i) == b'$' {
                    i += 1;
                }
                if self.at(i) != b'"' {
                    return refuse;
                }
                return match self.string_end(s) {
                    Some(end) => Ok((Str, end, 0, false)),
                    None => refuse,
                };
            }
            b'\'' => match self.char_end(s) {
                Some(end) => return Ok((Char, end, 0, false)),
                None => return refuse,
            },
            b'#' if s == 0 || (s == 3 && self.src.starts_with("\u{feff}".as_bytes())) => {
                if next == b'!' {
                    (Shebang, 2)
                } else {
                    return refuse;
                }
            }
            b'(' => (LParen, 1),
            b')' => (RParen, 1),
            b'[' => (LBracket, 1),
            b']' => (RBracket, 1),
            b'{' => (LBrace, 1),
            b'}' => (RBrace, 1),
            b';' => (Semi, 1),
            b',' => (Comma, 1),
            b'@' => (At, 1),
            b'?' => match next {
                b'.' => (QuestionDot, 2),
                b':' => (Elvis, 2),
                _ => (Question, 1),
            },
            b':' => {
                if next == b':' {
                    (ColonColon, 2)
                } else {
                    (Colon, 1)
                }
            }
            b'.' => {
                if next == b'.' {
                    if self.at(s + 2) == b'<' { (DotDotLt, 3) } else { (DotDot, 2) }
                } else {
                    (Dot, 1)
                }
            }
            b'=' => match (next, self.at(s + 2)) {
                (b'=', b'=') => (EqEqEq, 3),
                (b'=', _) => (EqEq, 2),
                _ => (Eq, 1),
            },
            b'!' => match (next, self.at(s + 2)) {
                (b'=', b'=') => (NeEq, 3),
                (b'=', _) => (Ne, 2),
                (b'!', _) => (BangBang, 2),
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
            b'&' => match next {
                b'&' => (AmpAmp, 2),
                _ => (Amp, 1),
            },
            b'|' if next == b'|' => (PipePipe, 2),
            b'<' => match next {
                b'=' => (Le, 2),
                _ => (Lt, 1),
            },
            b'>' => match next {
                b'=' => (Ge, 2),
                _ => (Gt, 1),
            },
            c if c >= 0x80 => {
                let (end, _) = self.name_end(s)?;
                if end == s {
                    return refuse;
                }
                return Ok((Name, end, 0, true));
            }
            _ => return refuse,
        };
        Ok((kind, s + len, 0, false))
    }

    /// The end of a name from `s` (`[\p{L}_][\p{L}_\p{Nd}]*`): ASCII word bytes a block a step, a
    /// character past ASCII by its class (refused where the class is not certain). `s` itself where
    /// no name starts there; and whether a character past ASCII is in it.
    pub fn name_end(&self, s: usize) -> R<(usize, bool)> {
        let c = self.at(s);
        if c.is_ascii_digit() || (c < 0x80 && !block::is_word(c)) {
            return Ok((s, false));
        }
        let mut i = s;
        let mut wide = false;
        loop {
            i = self.blocks.word_end(i);
            if self.at(i) < 0x80 || i >= self.src.len() {
                return Ok((i, wide));
            }
            let Some((ch, n)) = decode(self.src, i) else { return Err(Refusal::Lex(i as u32)) };
            match letter(ch) {
                Some(true) => {}
                Some(false) => return Ok((i, wide)),
                None => return Err(Refusal::Lex(i as u32)),
            }
            wide = true;
            i += n;
        }
    }

    /// `DEC_DIGITS` from `i` (`[0-9]+` runs with runs of `_` between them): its end, `None` where no
    /// digit starts it.
    fn digits(&self, mut i: usize, digit: &Class) -> Option<usize> {
        if !digit[self.at(i) as usize] {
            return None;
        }
        loop {
            i = swar::class_end(self.src, i, digit);
            let k = swar::run_end(self.src, i, b'_');
            if k > i && digit[self.at(k) as usize] {
                i = k;
                continue;
            }
            return Some(i);
        }
    }

    /// An exponent from `i` (`[eE][+-]?DEC_DIGITS`): its end.
    fn exponent(&self, i: usize) -> Option<usize> {
        if !matches!(self.at(i), b'e' | b'E') {
            return None;
        }
        let k = if matches!(self.at(i + 1), b'+' | b'-') { i + 2 } else { i + 1 };
        self.digits(k, &DEC)
    }

    /// A number at `s` (a digit, or a `.` before one): the longest of the grammar's literals. A
    /// `long_literal`'s `L` and an `unsigned_literal`'s suffix are tokens of their own.
    fn number(&self, s: usize) -> R<(T, usize, u8, bool)> {
        let (c0, c1) = (self.at(s), self.at(s + 1));
        if c0 == b'0' && matches!(c1, b'x' | b'X') {
            if let Some(e) = self.digits(s + 2, &HEX) {
                return Ok((T::Hex, e, 0, false));
            }
        }
        if c0 == b'0' && matches!(c1, b'b' | b'B') {
            if let Some(e) = self.digits(s + 2, &BIN) {
                return Ok((T::Bin, e, 0, false));
            }
        }
        let mut best = (T::Int, s);
        let int = if c0 == b'.' { None } else { self.digits(s, &DEC) };
        if let Some(e) = int {
            best = (T::Int, e);
            if let Some(x) = self.exponent(e) {
                let x = if matches!(self.at(x), b'f' | b'F') { x + 1 } else { x };
                best = (T::Real, x);
            } else if matches!(self.at(e), b'f' | b'F') {
                best = (T::Real, e + 1);
            }
        }
        // `DEC_DIGITS? '.' DEC_DIGITS REAL_EXPONENT? [fF]?`.
        let dot = int.unwrap_or(s);
        if self.at(dot) == b'.' {
            if let Some(f) = self.digits(dot + 1, &DEC) {
                let x = self.exponent(f).unwrap_or(f);
                let x = if matches!(self.at(x), b'f' | b'F') { x + 1 } else { x };
                if x > best.1 {
                    best = (T::Real, x);
                }
            }
        }
        if best.1 == s {
            return Err(Refusal::Lex(s as u32));
        }
        Ok((best.0, best.1, 0, false))
    }

    /// The end of a string literal from its first `$` or `"` at `s`: past its closer. An
    /// interpolation's (`${…}`) brackets, strings and characters are passed. `None` where the file
    /// ends first.
    pub fn string_end(&self, s: usize) -> Option<usize> {
        let mut i = s;
        while self.at(i) == b'$' {
            i += 1;
        }
        let triple = self.src[i..].starts_with(b"\"\"\"");
        i += if triple { 3 } else { 1 };
        loop {
            i = self.blocks.string_stop(i);
            match self.src.get(i)? {
                b'"' if !triple => return Some(i + 1),
                b'"' if self.src[i..].starts_with(b"\"\"\"") => {
                    // The closer takes every quote of the run.
                    let mut k = i + 3;
                    while self.at(k) == b'"' {
                        k += 1;
                    }
                    return Some(k);
                }
                b'"' => i += 1,
                // In a raw string `\\` escapes nothing but a `$` (the scanner's reading).
                b'\\' if triple => i += if self.at(i + 1) == b'"' { 1 } else { 2 },
                b'\\' => {
                    if i + 1 >= self.src.len() {
                        return None;
                    }
                    i += 2
                }
                b'$' if self.at(i + 1) == b'{' => i = self.interpolation_end(i + 2)?,
                _ => i += 1,
            }
        }
    }

    /// From inside an interpolation (past its `${`): past its `}`, nested brackets, strings,
    /// characters and comments passed.
    fn interpolation_end(&self, mut i: usize) -> Option<usize> {
        let mut depth = 0u32;
        loop {
            match self.src.get(i)? {
                b'{' => depth += 1,
                b'}' if depth == 0 => return Some(i + 1),
                b'}' => depth -= 1,
                b'"' => {
                    i = self.string_end(i)?;
                    continue;
                }
                b'$' => {
                    let mut k = i;
                    while self.at(k) == b'$' {
                        k += 1;
                    }
                    if self.at(k) == b'"' {
                        i = self.string_end(i)?;
                        continue;
                    }
                }
                b'\'' => {
                    i = self.char_end(i)?;
                    continue;
                }
                b'/' if matches!(self.at(i + 1), b'/' | b'*') => {
                    let x = self.comment(i).ok()??;
                    i = x.end as usize;
                    continue;
                }
                _ => {}
            }
            i += 1;
        }
    }

    /// A character literal from `s` (its `'`): one character but `\n`, `\r`, `'` and `\`, or an
    /// escape (`\uXXXX`, `\` and one of `tbrn'"\$`); then `'`. Its end.
    pub fn char_end(&self, s: usize) -> Option<usize> {
        let i = s + 1;
        let e = match self.at(i) {
            b'\\' => self.char_escape_end(i)?,
            b'\n' | b'\r' | b'\'' => return None,
            _ if i >= self.src.len() => return None,
            c if c >= 0x80 => i + decode(self.src, i)?.1,
            _ => i + 1,
        };
        (self.at(e) == b'\'').then_some(e + 1)
    }

    /// A character escape from `i` (its `\`): `\u` and 4 hex digits, or `\` and one of `tbrn'"\$`.
    pub fn char_escape_end(&self, i: usize) -> Option<usize> {
        match self.at(i + 1) {
            b'u' => (i + 2..i + 6).all(|k| self.at(k).is_ascii_hexdigit()).then_some(i + 6),
            b't' | b'b' | b'r' | b'n' | b'\'' | b'"' | b'\\' | b'$' => Some(i + 2),
            _ => None,
        }
    }

    // ---- The external scanner's automatic semicolon.

    /// Whether the word `w` starts at `i` and ends there (no word byte after it).
    fn word_at(&self, i: usize, w: &[u8]) -> bool {
        self.src[i.min(self.src.len())..].starts_with(w) && !wordc(self.at(i + w.len()))
    }

    /// From `i`: white space and comments passed (`skip_whitespace_and_comments`); `None` at a
    /// bare `/`.
    fn past_space_and_comments(&self, mut i: usize) -> Option<usize> {
        loop {
            while space(self.at(i)) {
                i += 1;
            }
            if self.at(i) != b'/' {
                return Some(i);
            }
            match self.at(i + 1) {
                b'/' => {
                    i += 2;
                    while i < self.src.len() && !matches!(self.src[i], b'\n' | b'\r') {
                        i += 1;
                    }
                }
                b'*' => {
                    // To the `*/` (with nesting), or the file's end.
                    i += 2;
                    let mut depth = 1u32;
                    while depth > 0 && i < self.src.len() {
                        if self.src[i] == b'*' {
                            i += 1;
                            if self.at(i) == b'/' {
                                i += 1;
                                depth -= 1;
                            }
                        } else if self.src[i] == b'/' {
                            i += 1;
                            if self.at(i) == b'*' {
                                i += 1;
                                depth += 1;
                            }
                        } else {
                            i += 1;
                        }
                    }
                }
                _ => return None,
            }
        }
    }

    /// `else` at `i` followed by `->` (a when entry's `else`).
    fn else_arrow(&self, i: usize) -> bool {
        self.past_space_and_comments(i + 4).is_some_and(|k| self.at(k) == b'-' && self.at(k + 1) == b'>')
    }

    /// A visibility modifier at `i`, blanks, then `constructor`.
    fn modifier_then_constructor(&self, i: usize) -> bool {
        let mut k = i;
        while wordc(self.at(k)) && k - i < 19 {
            k += 1;
        }
        if !matches!(&self.src[i..k], b"public" | b"private" | b"protected" | b"internal") {
            return false;
        }
        while matches!(self.at(k), b' ' | b'\t') {
            k += 1;
        }
        self.word_at(k, b"constructor")
    }

    /// Annotations at `i`, then an optional visibility modifier, then `constructor`.
    fn annotation_then_constructor(&self, mut i: usize) -> bool {
        while self.at(i) == b'@' {
            i += 1;
            if !wordc(self.at(i)) {
                return false;
            }
            while wordc(self.at(i)) {
                i += 1;
            }
            while self.at(i) == b'.' {
                i += 1;
                if !wordc(self.at(i)) {
                    break;
                }
                while wordc(self.at(i)) {
                    i += 1;
                }
            }
            if self.at(i) == b'(' {
                let mut depth = 1u32;
                i += 1;
                while depth > 0 && i < self.src.len() && self.src[i] != 0 {
                    if self.src[i] == b'"' {
                        i += 1;
                        while i < self.src.len() && self.src[i] != b'"' && self.src[i] != 0 {
                            if self.src[i] == b'\\' {
                                i += 1;
                            }
                            i += 1;
                        }
                        if self.at(i) == b'"' {
                            i += 1;
                        }
                    } else {
                        if self.src[i] == b'(' {
                            depth += 1;
                        } else if self.src[i] == b')' {
                            depth -= 1;
                        }
                        i += 1;
                    }
                }
            }
            while space(self.at(i)) {
                i += 1;
            }
        }
        if wordc(self.at(i)) && self.at(i) != b'c' {
            return self.modifier_then_constructor(i);
        }
        self.word_at(i, b"constructor")
    }

    /// The automatic semicolon after a token ending at `from`, where the parser's state takes one
    /// (`scan_automatic_semicolon`): the answer, and where the scanner was last asked (a comment on
    /// the token's line is lexed as an extra and the scanner asked again after it, as tree-sitter
    /// does). A block comment the scanner reads on the next line is lost to the tree (its token's
    /// start moved past it by the scanner's `skip`): `lost` gets its start and the zero-width
    /// comment's place, where the scanner leaves one.
    pub fn asi(&self, mut from: usize, cx: AsiCtx, lost: &mut Vec<(u32, u32)>) -> (Asi, u32) {
        let n = self.src.len();
        'call: loop {
            // The token's line.
            let mut i = from;
            loop {
                if i >= n {
                    return (Asi::Zero, from as u32);
                }
                match self.src[i] {
                    b';' => return (Asi::Semi(i as u32), from as u32),
                    b'\n' => {
                        i += 1;
                        break;
                    }
                    b'\r' => {
                        i += 1;
                        if self.at(i) == b'\n' {
                            i += 1;
                        }
                        break;
                    }
                    c if space(c) => i += 1,
                    b'i' => return (if self.word_at(i, b"import") { Asi::Zero } else { Asi::No }, from as u32),
                    b'/' if matches!(self.at(i + 1), b'/' | b'*') => {
                        // The scanner declines; the comment is lexed; the scanner runs again after it.
                        let end = if self.at(i + 1) == b'/' {
                            let mut k = i + 2;
                            while k < n && !matches!(self.src[k], b'\n' | b'\r') {
                                k += 1;
                            }
                            k
                        } else {
                            match self.block_comment_end(i) {
                                Some(e) => e,
                                None => return (Asi::No, from as u32),
                            }
                        };
                        from = end;
                        continue 'call;
                    }
                    _ => return (Asi::No, from as u32),
                }
            }
            // Past a line end: the next line's first byte decides.
            while space(self.at(i)) {
                i += 1;
            }
            let zero = (Asi::Zero, from as u32);
            let a = match self.at(i) {
                b',' | b'.' | b':' | b'*' | b'%' | b'>' | b'<' | b'=' | b'{' | b'[' | b'(' | b'?' | b'|' | b'&' => Asi::No,
                b'/' => match self.at(i + 1) {
                    b'/' => {
                        let mut k = i + 2;
                        while k < n && !matches!(self.src[k], b'\n' | b'\r') {
                            k += 1;
                        }
                        let Some(k) = self.past_space_and_comments(k) else { return (Asi::No, from as u32) };
                        match self.at(k) {
                            b'.' | b',' | b':' | b'*' | b'%' | b'>' | b'<' | b'=' | b'{' | b'[' | b'(' | b'?' | b'|' | b'&' | b'/' => Asi::No,
                            b'!' if self.at(k + 1) == b'=' => Asi::No,
                            b'e' if self.word_at(k, b"else") && !self.else_arrow(k) => Asi::No,
                            b'a' if self.word_at(k, b"as") => Asi::No,
                            b'w' if self.word_at(k, b"where") => Asi::No,
                            b'c' if self.word_at(k, b"catch") => Asi::No,
                            b'b' if cx.by && self.word_at(k, b"by") => Asi::No,
                            b'f' if self.word_at(k, b"finally") => Asi::No,
                            _ => return zero,
                        }
                    }
                    b'*' => {
                        // The scanner reads the comment through; what follows it decides. Where
                        // it marks the token's end past the comment, the comment is lost: a
                        // zero-width comment there (and the scanner asked again from there), or a
                        // semicolon there.
                        let Some(end) = self.block_comment_end(i) else { return (Asi::No, from as u32) };
                        let mut k = end;
                        while space(self.at(k)) {
                            k += 1;
                        }
                        let comment = match self.at(k) {
                            b'.' | b',' | b':' | b'%' | b'>' | b'<' | b'=' | b'{' | b'[' | b'(' | b'?' | b'|' | b'&' | b'/' | b'*' => true,
                            b'!' => self.at(k + 1) == b'=',
                            b'e' if self.word_at(k, b"else") => !self.else_arrow(k),
                            b'a' if self.word_at(k, b"as") => true,
                            b'w' if self.word_at(k, b"where") => true,
                            b'b' if cx.by && self.word_at(k, b"by") => true,
                            b'e' | b'a' | b'w' => false,
                            b'b' if cx.by => false,
                            _ => return zero,
                        };
                        if comment {
                            lost.push((i as u32, k as u32));
                            from = k;
                            continue 'call;
                        }
                        lost.push((i as u32, u32::MAX));
                        return (Asi::Swallow(k as u32), k as u32);
                    }
                    _ => Asi::No,
                },
                b'+' | b'-' => return zero,
                b'!' => {
                    if self.at(i + 1) == b'=' {
                        Asi::No
                    } else {
                        return zero;
                    }
                }
                b'b' if cx.by && self.word_at(i, b"by") => Asi::No,
                b'e' if self.word_at(i, b"else") && !self.else_arrow(i) => Asi::No,
                b'a' if self.word_at(i, b"as") => Asi::No,
                b'w' if self.word_at(i, b"where") => Asi::No,
                b'i' | b'p' if cx.ctor && self.modifier_then_constructor(i) => Asi::No,
                b'c' if cx.ctor => {
                    if self.word_at(i, b"constructor") {
                        Asi::Ctor
                    } else {
                        return zero;
                    }
                }
                b'c' if self.word_at(i, b"catch") => Asi::No,
                b'f' if self.word_at(i, b"finally") => Asi::No,
                b'@' if cx.ctor && self.annotation_then_constructor(i) => Asi::No,
                b';' => Asi::Semi(i as u32),
                _ => return zero,
            };
            return (a, from as u32);
        }
    }
}

#[cfg(test)]
mod block_lexer_tests {
    use super::*;

    type Run = (Vec<String>, Vec<(u32, u32, u32)>, Option<Refusal>);

    /// The tokens, the comments and the refusal from `at`, by the token loop (`token` a token at
    /// a time) and by the block lexer (`lex_run`).
    fn both(src: &[u8], at: usize) -> [Run; 2] {
        let lx = Lexer::new(src);
        let one = {
            let (mut toks, mut extras, mut at) = (Vec::new(), Vec::new(), at);
            let r = loop {
                match lx.token(at, &mut extras) {
                    Ok(t) => {
                        toks.push(format!("{t:?}"));
                        if t.kind == T::Eof {
                            break None;
                        }
                        at = t.end as usize;
                    }
                    Err(e) => break Some(e),
                }
            };
            (toks, extras, r)
        };
        let run = {
            let (mut toks, mut extras, mut at) = (Vec::new(), Vec::new(), at);
            let r = loop {
                match lx.lex_run(at, &mut toks, 3, &mut extras) {
                    Ok(next) => {
                        if toks.last().is_some_and(|t: &Tok| t.kind == T::Eof) {
                            break None;
                        }
                        at = next;
                    }
                    Err(e) => break Some(e),
                }
            };
            (toks.iter().map(|t| format!("{t:?}")).collect(), extras, r)
        };
        [one, run]
    }

    #[test]
    fn the_block_lexer_is_the_token_loop() {
        // Texts of the bytes the lexer treats apart, long enough to cross blocks.
        let alphabet: &[&[u8]] = &[b"a", b"bc", b"_x9", b"fun", b"val", b"0", b"12", b"1.5", b" ", b"  ", b"\n", b"\r\n", b"\t", b"//c\n", b"/*c*/", b"/*\n*/", b"\"s\"", b"\"$a\"", b"'c'", b"`n`", b".", b"..", b"?", b"?.", b"?:", b":", b"::", b"=", b"==", b"!", b"!!", b"+", b"++", b"-", b"->", b"*", b"/", b"%", b"&", b"&&", b"|", b"||", b"<", b">", b"(", b")", b"[", b"]", b"{", b"}", b";", b",", b"@", b"$", b"\xc3\xa9", b"\xe2\x80\x83", b"\xe3\x81\x82", b"#"];
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for round in 0..3000 {
            let n = (next() % 200) as usize;
            let mut src = Vec::new();
            while src.len() < n {
                src.extend_from_slice(alphabet[(next() % alphabet.len() as u64) as usize]);
            }
            let starts: Vec<usize> = if round % 10 == 0 { (0..=src.len()).collect() } else { vec![0] };
            for at in starts {
                let [one, run] = both(&src, at);
                assert_eq!(one, run, "{:?} from {at}", String::from_utf8_lossy(&src));
            }
        }
    }
}
