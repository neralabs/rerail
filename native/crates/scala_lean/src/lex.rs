//! The lexer: tree-sitter-scala's internal tokens, read where the parser asks. It keeps no state:
//! the parser asks for the next token at a byte (`Lexer::token`). The external scanner's tokens
//! (layout, strings' pieces, postfix operators) are `scan`'s, asked by the parser where its state
//! takes them.
//!
//! A word is an `Id` with its word code (`w::…`; 0: none) from one probe of the keyword switch. The
//! grammar's reserved words (`w::is_reserved`) are keywords wherever they stand; any other keyword
//! is one where the parse state takes it (keyword extraction), so the parser decides.
//!
//! An operator is a run of operator characters (`op_char`; a `/` before `/` or `*` starts a
//! comment), its code the reserved spelling it is (`=`, `=>`, `<-`, …) or 0, and its precedence
//! class the one its first and last characters give (`op_class`, SLS 6.12.3).

use crate::block::{Blocks, Lexed};
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

/// A token's kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum T {
    Eof,
    /// A word: `_alpha_identifier` (its word code in `Tok::code`).
    Id,
    /// A back-quoted name.
    BackId,
    /// A run of operator characters (its reserved spelling in `Tok::code`, `op::…`).
    Op,
    Int,
    Float,
    /// A character literal.
    Char,
    /// `'` not opening a character literal (a quote, a symbol).
    Quote,
    /// A string's opening `"` or `"""` (its pieces are read by the parser).
    Str,
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Comma,
    Semi,
    Dot,
}

#[derive(Clone, Copy, Debug)]
pub struct Tok {
    pub kind: T,
    /// A word's code (`w::…`) or an operator's (`op::…`); 0: none.
    pub code: u8,
    /// A line end in the white space before the token (outside comments).
    pub nl: bool,
    pub start: u32,
    pub end: u32,
}

/// Word codes: every word the grammar or the scanner reads as a keyword somewhere.
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
        // The reserved words (`reserved.global`), in their order there: codes 1 to 38.
        ABSTRACT = b"abstract", CASE = b"case", CLASS = b"class", DEF = b"def", DO = b"do",
        ENUM = b"enum", EXPORT = b"export", EXTENDS = b"extends", FALSE = b"false", FINAL = b"final",
        FINALLY = b"finally", FOR = b"for", GIVEN = b"given", IMPLICIT = b"implicit",
        IMPORT = b"import", LAZY = b"lazy", MACRO = b"macro", MATCH = b"match", NEW = b"new",
        NULL = b"null", OBJECT = b"object", OVERRIDE = b"override", PACKAGE = b"package",
        PRIVATE = b"private", PROTECTED = b"protected", RETURN = b"return", SEALED = b"sealed",
        SUPER = b"super", THIS = b"this", THROW = b"throw", TRAIT = b"trait", TRUE = b"true",
        TRY = b"try", TYPE = b"type", VAL = b"val", VAR = b"var", WHILE = b"while", YIELD = b"yield",
        // Keywords where the state takes them.
        IF = b"if", ELSE = b"else", THEN = b"then", WITH = b"with", CATCH = b"catch",
        FORSOME = b"forSome", AS = b"as", USING = b"using", DERIVES = b"derives", USES = b"uses",
        END = b"end", EXTENSION = b"extension", UNDERSCORE = b"_", CAP = b"cap", ONLY = b"only",
        EXCEPT = b"except", RD = b"rd", RAW = b"raw",
        // The scanner's soft modifiers.
        INLINE = b"inline", OPAQUE = b"opaque", OPEN = b"open", INFIX = b"infix",
        TRANSPARENT = b"transparent", ERASED = b"erased", INTO = b"into", TRACKED = b"tracked",
        UPDATE = b"update", CONSUME = b"consume",
    }

    /// Whether word `code` is reserved: a keyword wherever it stands.
    #[inline(always)]
    pub const fn is_reserved(code: u8) -> bool {
        code >= ABSTRACT && code <= YIELD
    }
}

rerail_simd8::keyword_switch!(
    /// The keyword switch (`rerail_simd8::words`) over `w::TEXT`: a word's code is its index there.
    static WORDS = w::TEXT, fold: false
);

/// Operator codes: the spellings the grammar reads as tokens of their own.
#[allow(dead_code)]
pub mod op {
    pub const NONE: u8 = 0;
    pub const COLON: u8 = 1;
    pub const EQ: u8 = 2;
    pub const ARROW: u8 = 3;
    pub const LARROW: u8 = 4;
    pub const SUBTYPE: u8 = 5;
    pub const SUPERTYPE: u8 = 6;
    pub const VIEW: u8 = 7;
    pub const HASH: u8 = 8;
    pub const AT: u8 = 9;
    pub const TYPE_LAMBDA: u8 = 10;
    pub const QARROW: u8 = 11;
    pub const STAR: u8 = 12;
    pub const PIPE: u8 = 13;
    pub const MINUS: u8 = 14;
    pub const PLUS: u8 = 15;
    pub const BANG: u8 = 16;
    pub const TILDE: u8 = 17;
    pub const THIN: u8 = 18;
    pub const QTHIN: u8 = 19;
    pub const CARET: u8 = 20;
    pub const QUESTION: u8 = 21;

    /// The code of an operator's text.
    pub fn code(s: &[u8]) -> u8 {
        match s {
            b":" => COLON,
            b"=" => EQ,
            b"=>" => ARROW,
            b"<-" => LARROW,
            b"<:" => SUBTYPE,
            b">:" => SUPERTYPE,
            b"<%" => VIEW,
            b"#" => HASH,
            b"@" => AT,
            b"=>>" => TYPE_LAMBDA,
            b"?=>" => QARROW,
            b"*" => STAR,
            b"|" => PIPE,
            b"-" => MINUS,
            b"+" => PLUS,
            b"!" => BANG,
            b"~" => TILDE,
            b"->" => THIN,
            b"?->" => QTHIN,
            b"^" => CARET,
            b"?" => QUESTION,
            _ => NONE,
        }
    }

    /// Whether code `c` is one of the grammar's reserved operators (never an operator name).
    #[inline(always)]
    pub const fn is_reserved(c: u8) -> bool {
        matches!(c, COLON | EQ | LARROW | VIEW | HASH | AT | TYPE_LAMBDA | QARROW)
    }
}

/// The word `src[i..end]`'s code (0: none): one probe of the keyword switch.
#[inline(always)]
pub fn word(src: &[u8], i: usize, end: usize) -> u8 {
    WORDS.find(src, i, end) as u8
}

/// Digit classes (`swar::class_end`, 8 bytes a step).
static DEC: Class = swar::class_of(b"0123456789");
static HEX: Class = swar::class_of(b"0123456789abcdefABCDEF");
static BIN_: Class = swar::class_of(b"01_");

/// Bytes that start a name (`[A-Za-z_]`; `$` and one past ASCII go the long way).
static NAME_START: [bool; 256] = {
    let mut t = [false; 256];
    let mut c = 0;
    while c < 256 {
        t[c] = (c as u8).is_ascii_alphabetic() || c == b'_' as usize;
        c += 1;
    }
    t
};

// The bytes after a one-byte token that make it longer, or another (`FOLLOW`'s bits, a token's
// `ONE`): an operator character, a byte past ASCII, a digit.
const F_OP: u8 = 1;
const F_WIDE: u8 = 2;
const F_DIGIT: u8 = 4;

static FOLLOW: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut c = 0;
    while c < 256 {
        t[c] = if c >= 0x80 {
            F_WIDE
        } else if op_char(c as u8) {
            F_OP
        } else if (c as u8).is_ascii_digit() {
            F_DIGIT
        } else {
            0
        };
        c += 1;
    }
    t
};

/// A one-byte token by its byte (`token_at`'s, an operator of one byte): its kind, and the next
/// bytes that make it another (`u8::MAX`: no one-byte token, or one read the long way).
static ONE: [(T, u8); 256] = {
    let mut t = [(T::Eof, u8::MAX); 256];
    t[b'(' as usize] = (T::LParen, 0);
    t[b')' as usize] = (T::RParen, 0);
    t[b'[' as usize] = (T::LBracket, 0);
    t[b']' as usize] = (T::RBracket, 0);
    t[b'{' as usize] = (T::LBrace, 0);
    t[b'}' as usize] = (T::RBrace, 0);
    t[b',' as usize] = (T::Comma, 0);
    t[b';' as usize] = (T::Semi, 0);
    t[b'.' as usize] = (T::Dot, F_DIGIT);
    let mut c = 0;
    while c < 128 {
        if op_char(c as u8) {
            t[c] = (T::Op, F_OP | F_WIDE);
        }
        c += 1;
    }
    t
};

/// An ASCII operator character (`/` included: the caller decides whether it opens a comment).
#[inline(always)]
pub const fn op_char(c: u8) -> bool {
    matches!(c, b'-' | b'!' | b'#' | b'%' | b'&' | b'*' | b'+' | b'/' | b'\\' | b':' | b'<' | b'=' | b'>' | b'?' | b'@' | b'^' | b'|' | b'~')
}

/// An infix operator's precedence (6 to 16) and whether it is right-associative: the class its first
/// character gives (`|` 8, `^` 9, `&` 10, `=` `!` 11, `<` `>` 12, `:` 13, `+` `-` 14, `*` `/` `%` 15,
/// any other 16), right-associative where it ends in `:`; an assignment operator (`+=`, `<<=`, …)
/// is 6.
pub fn op_class(s: &[u8]) -> (u8, bool) {
    let (first, last) = (s[0], s[s.len() - 1]);
    if last == b'=' && s.len() >= 2 {
        let assign = match first {
            b'=' => false,
            b'<' | b'>' | b'!' => s.len() >= 3,
            _ => true,
        };
        if assign {
            return (6, false);
        }
    }
    let right = last == b':' && s.len() >= 2;
    let p = match first {
        b'|' => 8,
        b'^' => 9,
        b'&' => 10,
        b'=' | b'!' => 11,
        b'<' | b'>' => 12,
        b':' => 13,
        b'+' | b'-' => 14,
        b'*' | b'/' | b'%' => 15,
        _ => 16,
    };
    (p, right)
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
            if c < 0x80 && c != b'/' && !crate::block::is_space(c) {
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
                if let Some((c, _)) = decode(self.src, at) {
                    if c.is_whitespace() {
                        return Err(Refusal::Lex(at as u32));
                    }
                }
            }
            match self.comment(at)? {
                None => return Ok((at, nl)),
                Some((end, true)) => {
                    extras.push((crate::tw::key(crate::K::BlockComment, 0, true), at as u32, end as u32));
                    at = end;
                }
                Some((end, false)) => {
                    self.line_comment(at, end, extras)?;
                    at = end;
                }
            }
        }
    }

    /// A line comment from `at` to `end`: its node, its `//`, and a `using` directive's nodes
    /// (`//> using key value`).
    fn line_comment(&self, at: usize, end: usize, extras: &mut Vec<(u32, u32, u32)>) -> R<()> {
        use crate::tw::{EXTRA_DEPTH, key};
        use crate::K;
        extras.push((key(K::Comment, 0, true), at as u32, end as u32));
        extras.push((key(K::TSlashSlash, 0, false) | 1 << EXTRA_DEPTH, at as u32, at as u32 + 2));
        if self.at(at + 2) == b'>' {
            // `//>` then `using`: a directive (`using_directive`: from the `>`, its key the word after
            // `using`, its value the rest of the line from the key's end).
            let i = self.blocks.blank_end(at + 3);
            if self.src[i..end].starts_with(b"using") && !self.at(i + 5).is_ascii_alphanumeric() {
                let k = self.blocks.blank_end(i + 5);
                // The key: to the next blank, a word at a time.
                let mut ke = k;
                while ke < end && !matches!(self.at(ke), b' ' | b'\t') {
                    ke = self.blocks.word_end(ke).max(ke + 1).min(end);
                }
                if k == i + 5 || k >= end || ke >= end {
                    return Err(Refusal::Lex(at as u32));
                }
                extras.push((key(K::UsingDirective, 0, false) | 1 << EXTRA_DEPTH, at as u32 + 2, end as u32));
                extras.push((key(K::UsingDirectiveKey, 0, false) | 2 << EXTRA_DEPTH, k as u32, ke as u32));
                extras.push((key(K::UsingDirectiveValue, 0, false) | 2 << EXTRA_DEPTH, ke as u32, end as u32));
            }
        }
        Ok(())
    }

    /// The comment starting at `at`, if one does: its end, and whether it is a block comment. An
    /// unclosed `/*` refused. Block comments nest.
    #[inline]
    pub fn comment(&self, at: usize) -> R<Option<(usize, bool)>> {
        if self.at(at) != b'/' {
            return Ok(None);
        }
        match self.at(at + 1) {
            b'/' => Ok(Some((self.line_end(at + 2), false))),
            b'*' => match self.block_comment_end(at) {
                Some(end) => Ok(Some((end, true))),
                None => Err(Refusal::Lex(at as u32)),
            },
            _ => Ok(None),
        }
    }

    /// The next `\n` from `i` (a `\r` is a line comment's text: `.*` reads it), or the end.
    pub fn line_end(&self, i: usize) -> usize {
        let mut end = self.blocks.eol(i);
        while end < self.src.len() && self.src[end] != b'\n' {
            end = self.blocks.eol(end + 1);
        }
        end
    }

    /// The end of the (nested) block comment at `at` (its `/*`); `None` where the file ends first.
    pub fn block_comment_end(&self, at: usize) -> Option<usize> {
        let mut depth = 1u32;
        let mut i = at + 2;
        loop {
            i = self.blocks.star(i);
            if i >= self.src.len() {
                return None;
            }
            match (self.src[i], self.at(i + 1)) {
                (b'*', b'/') => {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                (b'/', b'*') => {
                    depth += 1;
                    i += 2;
                }
                _ => i += 1,
            }
        }
    }

    /// Tokens from `at` on into `out`, a block at a time (`Blocks::lex_blocks`): names and
    /// one-byte punctuation and operators straight from the block's masks and two tables,
    /// comments in place, every other token by `token_at` from its start (by `token` where a
    /// character past ASCII starts it). Returns where the next is to be read from; at least `min`
    /// tokens are made unless the text ends.
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
                let tok = |kind, code, e: usize| Ok(Lexed::Tok(Tok { kind, code, nl, start: pos as u32, end: e as u32 }, e));
                if NAME_START[c as usize] {
                    // A name of ASCII bytes (past the block too), ended before ASCII other than
                    // `$`, and not before an operator suffix (`foo_+`: `name_end`).
                    let e = if run == usize::MAX { self.blocks.word_end(pos) } else { pos + run };
                    let d = self.at(e);
                    if d < 0x80 && d != b'$' && !(src[e - 1] == b'_' && op_char(d)) {
                        return tok(T::Id, word(src, pos, e), e);
                    }
                } else {
                    let (kind, bad) = ONE[c as usize];
                    if bad != u8::MAX && FOLLOW[self.at(pos + 1) as usize] & bad == 0 {
                        let code = if kind == T::Op { op::code(&src[pos..pos + 1]) } else { 0 };
                        return tok(kind, code, pos + 1);
                    }
                    if c >= 0x80 {
                        // White space past ASCII is refused (`skip`): the long way.
                        return Ok(Lexed::Slow);
                    }
                    if c == b'/' {
                        match self.comment(pos)? {
                            None => {}
                            Some((end, true)) => {
                                extras.push((crate::tw::key(crate::K::BlockComment, 0, true), pos as u32, end as u32));
                                return Ok(Lexed::Extra(end));
                            }
                            Some((end, false)) => {
                                self.line_comment(pos, end, extras)?;
                                return Ok(Lexed::Extra(end));
                            }
                        }
                    }
                }
                // Any other token from its first byte, as `token` reads it there.
                let (kind, code, e) = self.token_at(pos)?;
                if e <= pos {
                    return Err(Refusal::Lex(pos as u32));
                }
                tok(kind, code, e)
            },
            |extras, gap, nl| {
                let mut t = self.token(gap, extras)?;
                t.nl |= nl;
                Ok((t, t.end as usize, t.kind == T::Eof))
            },
        )
    }

    /// The token at or after `at` (white space and comments passed, the comments pushed to
    /// `extras`).
    pub fn token(&self, at: usize, extras: &mut Vec<(u32, u32, u32)>) -> R<Tok> {
        let (s, nl) = self.skip(at, extras)?;
        let (kind, code, e) = self.token_at(s)?;
        if e <= s && kind != T::Eof {
            return Err(Refusal::Lex(s as u32));
        }
        Ok(Tok { kind, code, nl, start: s as u32, end: e as u32 })
    }

    /// The end of the string whose quote is at `s`, its whole extent one token (so that a look
    /// ahead passes over it): plain or interpolated (an identifier right before it), single-line
    /// or multiline; an interpolation's `${…}` passed by its tokens, `depth` strings deep.
    fn string_end(&self, s: usize, depth: u32) -> R<usize> {
        if depth > 8 {
            return Err(Refusal::Lex(s as u32));
        }
        let multi = self.src[s..].starts_with(b"\"\"\"");
        let interp = s > 0 && (self.at(s - 1).is_ascii_alphanumeric() || matches!(self.at(s - 1), b'_' | b'$'));
        let mut i = if multi { s + 3 } else { s + 1 };
        loop {
            let stop = self.blocks.string_stop(i);
            if stop >= self.src.len() || !multi && self.blocks.eol(i) < stop {
                return Err(Refusal::Lex(s as u32));
            }
            match self.src[stop] {
                b'"' if !multi => return Ok(stop + 1),
                b'"' if self.src[stop..].starts_with(b"\"\"\"") => {
                    let mut e = stop + 3;
                    while self.at(e) == b'"' {
                        e += 1;
                    }
                    return Ok(e);
                }
                b'"' => i = stop + 1,
                b'\\' if multi => i = stop + 1,
                b'\\' => i = stop + 2,
                _ if !interp => i = stop + 1,
                _ => match self.at(stop + 1) {
                    b'{' => {
                        // The interpolation's tokens to its `}`.
                        let mut d = 0u32;
                        let mut at = stop + 1;
                        let mut scratch = Vec::new();
                        loop {
                            let (b, _) = self.skip(at, &mut scratch)?;
                            let (k, _, e) = match self.at(b) {
                                b'"' => (T::Str, 0, self.string_end(b, depth + 1)?),
                                _ => self.token_at(b)?,
                            };
                            match k {
                                T::LBrace => d += 1,
                                T::RBrace => {
                                    d -= 1;
                                    if d == 0 {
                                        at = e;
                                        break;
                                    }
                                }
                                T::Eof => return Err(Refusal::Lex(s as u32)),
                                _ => {}
                            }
                            at = e;
                        }
                        i = at;
                    }
                    _ => i = stop + 1,
                },
            }
        }
    }

    fn token_at(&self, s: usize) -> R<(T, u8, usize)> {
        use T::*;
        let c = self.at(s);
        Ok(match c {
            0 if s >= self.src.len() => (Eof, 0, s),
            b'(' => (LParen, 0, s + 1),
            b')' => (RParen, 0, s + 1),
            b'[' => (LBracket, 0, s + 1),
            b']' => (RBracket, 0, s + 1),
            b'{' => (LBrace, 0, s + 1),
            b'}' => (RBrace, 0, s + 1),
            b',' => (Comma, 0, s + 1),
            b';' => (Semi, 0, s + 1),
            b'.' if self.at(s + 1).is_ascii_digit() => (Float, 0, self.number(s)?),
            b'.' => (Dot, 0, s + 1),
            b'"' => (Str, 0, self.string_end(s, 0)?),
            b'\'' => self.quote(s)?,
            b'`' => {
                let e = Some(swar::first_of2(self.src, s + 1, self.src.len(), b'`', b'\n')).filter(|&e| e < self.src.len());
                match e {
                    Some(e) if self.src[e] == b'`' && e > s + 1 => (BackId, 0, e + 1),
                    _ => return Err(Refusal::Lex(s as u32)),
                }
            }
            b'0'..=b'9' => {
                let e = self.number(s)?;
                let float = self.src[s..e].iter().any(|&b| matches!(b, b'.' | b'e' | b'E' | b'f' | b'F' | b'd' | b'D')) && !self.src[s..e].starts_with(b"0x") && !self.src[s..e].starts_with(b"0X");
                (if float { Float } else { Int }, 0, e)
            }
            c if c.is_ascii_alphabetic() || c == b'_' || c == b'$' => {
                let e = self.name_end(s)?;
                (Id, word(self.src, s, e), e)
            }
            c if op_char(c) => {
                let e = self.op_end(s)?;
                (Op, op::code(&self.src[s..e]), e)
            }
            c if c >= 0x80 => {
                let Some((ch, n)) = decode(self.src, s) else { return Err(Refusal::Lex(s as u32)) };
                if ch.is_alphabetic() && letter_sure(ch) {
                    let e = self.name_end(s)?;
                    (Id, 0, e)
                } else if math_op(ch) {
                    // A run of mathematical operators and ASCII operator characters (`⊥`, `≺:`,
                    // `_op_name`'s patterns), its precedence its first character's (the highest).
                    let mut e = s + n;
                    loop {
                        let c = self.at(e);
                        if op_char(c) && !(c == b'/' && matches!(self.at(e + 1), b'/' | b'*')) {
                            e += 1;
                        } else if c >= 0x80 {
                            match decode(self.src, e) {
                                Some((ch, k)) if math_op(ch) => e += k,
                                _ => break,
                            }
                        } else {
                            break;
                        }
                    }
                    (Op, op::NONE, e)
                } else {
                    return Err(Refusal::Lex(s as u32));
                }
            }
            _ => return Err(Refusal::Lex(s as u32)),
        })
    }

    /// The end of a name from `s`: its word run, and an operator suffix after a final `_`
    /// (`unary_!`, `x_=`). A character past ASCII in it is read where it is surely a letter.
    pub fn name_end(&self, s: usize) -> R<usize> {
        let mut e = s;
        loop {
            e = self.blocks.word_end(e);
            // `$` is a letter in Scala, outside the word mask.
            if self.at(e) == b'$' {
                e += 1;
                continue;
            }
            if self.at(e) >= 0x80 {
                match decode(self.src, e) {
                    Some((ch, n)) if ch.is_alphanumeric() && letter_sure(ch) => {
                        e += n;
                        continue;
                    }
                    // A mathematical operator after a name starts the next token (not after `_`).
                    Some((ch, _)) if math_op(ch) && self.src[e - 1] != b'_' => {}
                    Some((ch, _)) if !ch.is_alphanumeric() && !ch.is_whitespace() && !matches!(ch as u32, 0x2000..=0x206f) => {
                        // An operator character past ASCII after a name: refused.
                        return Err(Refusal::Lex(e as u32));
                    }
                    _ => {}
                }
            }
            break;
        }
        // An operator suffix after `_` (`foo_+`); a lone `_` is a wildcard (`_*` is two tokens).
        if e > s + 1 && self.src[e - 1] == b'_' && op_char(self.at(e)) {
            let c = self.at(e);
            if c != b'/' || !matches!(self.at(e + 1), b'/' | b'*') {
                return self.op_end(e);
            }
        }
        Ok(e)
    }

    /// The end of the operator run from `s`: a `/` before `/` or `*` opens a comment and ends it.
    pub fn op_end(&self, s: usize) -> R<usize> {
        let mut e = s;
        loop {
            let c = self.at(e);
            if op_char(c) {
                if c == b'/' && matches!(self.at(e + 1), b'/' | b'*') {
                    break;
                }
                e += 1;
            } else if c >= 0x80 {
                // Mathematical operators continue the run (`<≺`).
                match decode(self.src, e) {
                    Some((ch, k)) if math_op(ch) => e += k,
                    _ => break,
                }
            } else {
                break;
            }
        }
        if self.at(e) >= 0x80 {
            if let Some((ch, _)) = decode(self.src, e) {
                if !ch.is_alphanumeric() && !ch.is_whitespace() {
                    return Err(Refusal::Lex(e as u32));
                }
            }
        }
        Ok(e)
    }

    /// A number from `s` (a digit, or `.` before one): an integer (decimal, hex, binary, an `l`
    /// after) or a floating point literal. A separator (`_`) in a float's integer part is the
    /// scanner's token, read here alike.
    fn number(&self, s: usize) -> R<usize> {
        let src = self.src;
        let digits = |mut i: usize| -> usize {
            // `\d(_?\d)*`: runs of digits 8 bytes a step, a `_` between two digits.
            loop {
                i = swar::class_end(src, i, &DEC);
                if i > 0 && src[i - 1].is_ascii_digit() && self.at(i) == b'_' && self.at(i + 1).is_ascii_digit() {
                    i += 1;
                    continue;
                }
                return i;
            }
        };
        let mut i = s;
        if self.at(i) == b'0' && matches!(self.at(i + 1), b'x' | b'X') {
            i += 2;
            loop {
                i = swar::class_end(src, i, &HEX);
                if self.at(i) == b'_' && self.at(i + 1).is_ascii_hexdigit() {
                    i += 1;
                    continue;
                }
                break;
            }
            if matches!(self.at(i), b'l' | b'L') {
                i += 1;
            }
            return self.number_end(i);
        }
        if self.at(i) == b'0' && matches!(self.at(i + 1), b'b' | b'B') && matches!(self.at(i + 2), b'0' | b'1') {
            i += 2;
            i = swar::class_end(src, i, &BIN_);
            if matches!(self.at(i), b'l' | b'L') {
                i += 1;
            }
            return self.number_end(i);
        }
        let mut float = false;
        if self.at(i) != b'.' {
            i = digits(i);
        }
        if self.at(i) == b'.' && self.at(i + 1).is_ascii_digit() {
            float = true;
            i = digits(i + 1);
        }
        if matches!(self.at(i), b'e' | b'E') {
            let mut j = i + 1;
            if matches!(self.at(j), b'+' | b'-') {
                j += 1;
            }
            if self.at(j).is_ascii_digit() {
                float = true;
                i = digits(j);
            }
        }
        if matches!(self.at(i), b'f' | b'F' | b'd' | b'D') {
            i += 1;
        } else if !float && matches!(self.at(i), b'l' | b'L') {
            i += 1;
        }
        self.number_end(i)
    }

    /// A number's end, refused where a name's character touches it (`1x`, `0x`).
    fn number_end(&self, i: usize) -> R<usize> {
        let c = self.at(i);
        if c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80 {
            return Err(Refusal::Lex(i as u32));
        }
        Ok(i)
    }

    /// `'` at `s`: a character literal (`'a'`, `'\n'`, `'A'`, `''`), else a quote.
    fn quote(&self, s: usize) -> R<(T, u8, usize)> {
        let c = self.at(s + 1);
        if c == b'\\' {
            // An escape: `\` then a character, `u`s and 4 hex digits, or `x` and 2.
            let mut i = s + 2;
            match self.at(i) {
                b'u' | b'U' => {
                    while matches!(self.at(i), b'u' | b'U') {
                        i += 1;
                    }
                    if !(0..4).all(|k| self.at(i + k).is_ascii_hexdigit()) {
                        return Err(Refusal::Lex(s as u32));
                    }
                    i += 4;
                }
                b'x' => {
                    if !(1..3).all(|k| self.at(i + k).is_ascii_hexdigit()) {
                        return Err(Refusal::Lex(s as u32));
                    }
                    i += 3;
                }
                b'\n' | 0 => return Err(Refusal::Lex(s as u32)),
                _ => i += 1,
            }
            if self.at(i) != b'\'' {
                return Err(Refusal::Lex(s as u32));
            }
            return Ok((T::Char, 0, i + 1));
        }
        if c == b'\'' {
            return Ok((T::Char, 0, s + 2));
        }
        if c < 0x80 {
            if c != b'\n' && c != 0 && self.at(s + 2) == b'\'' {
                return Ok((T::Char, 0, s + 3));
            }
            return Ok((T::Quote, 0, s + 1));
        }
        match decode(self.src, s + 1) {
            Some((_, n)) if self.at(s + 1 + n) == b'\'' => Ok((T::Char, 0, s + 2 + n)),
            _ => Ok((T::Quote, 0, s + 1)),
        }
    }
}

/// Whether `c` (past ASCII) is a mathematical operator an operator name is made of: the arrows,
/// the mathematical operators and their supplements (`\p{Sm}` there). `⇒` and `←`, which the
/// grammar reads as `=>` and `<-`, are not.
fn math_op(c: char) -> bool {
    let u = c as u32;
    matches!(u, 0x2190..=0x21ff | 0x2200..=0x22ff | 0x27c0..=0x27ef | 0x2900..=0x2aff) && u != 0x21d2 && u != 0x2190
}

/// The character at `at` (a UTF-8 lead byte at or past 0x80) and its length.
pub fn decode(src: &[u8], at: usize) -> Option<(char, usize)> {
    let s = std::str::from_utf8(&src[at..(at + 4).min(src.len())]).or_else(|e| std::str::from_utf8(&src[at..at + e.valid_up_to()])).ok()?;
    let c = s.chars().next()?;
    Some((c, c.len_utf8()))
}

/// Whether `c` (past ASCII) is surely a letter to the identifier's pattern (`\p{L}` and `\p{Nl}`
/// in the scripts read for certain: Latin, Greek, Cyrillic, kana, CJK, Hangul, the modifier
/// letters).
fn letter_sure(c: char) -> bool {
    let u = c as u32;
    // The modifier letters the pattern names (U+02B0–02C1 are `\p{Lm}`, U+02E0–02E4 listed).
    matches!(u, 0xc0..=0x24f | 0x2b0..=0x2c1 | 0x2e0..=0x2e4 | 0x370..=0x3ff | 0x400..=0x4ff | 0x3040..=0x30ff | 0x4e00..=0x9fff | 0xac00..=0xd7a3)
        && u != 0xd7
        && u != 0xf7
        && u != 0x37e
        && u != 0x387
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
        let alphabet: &[&[u8]] = &[b"a", b"bc", b"_x9", b"def", b"val", b"foo_", b"x_=", b"a$b", b"$a", b"_", b"0", b"12", b"1.5", b"0x1F", b"1L", b" ", b"  ", b"\n", b"\r\n", b"\t", b"//c\n", b"//> using x\n", b"/*c*/", b"/*\n*/", b"/* /* */ */", b"\"s\"", b"\"\"\"t\"\"\"", b"'c'", b"'a", b"`n`", b".", b".5", b":", b"::", b"=", b"==", b"=>", b"<-", b"!", b"+", b"++", b"-", b"->", b"*", b"/", b"%", b"&", b"&&", b"|", b"^", b"~", b"<", b">", b"<:", b"#", b"@", b"?", b"\\", b"(", b")", b"[", b"]", b"{", b"}", b";", b",", b"\xc3\xa9", b"\xe2\x80\x83", b"\xe2\x86\x92", b"\xe2\x8a\xa5", b"\xe3\x81\x82"];
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
