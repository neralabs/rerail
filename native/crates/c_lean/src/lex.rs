//! The lexer: tree-sitter-c's and tree-sitter-cpp's lexers, read where the parser asks. It keeps no
//! state: the parser asks for the next token at a byte (`Lexer::token`), and reads itself the few
//! places its parse state lexes otherwise (a string's pieces, a directive's argument, `<…>` after
//! `#include`, a raw string).
//!
//! **Line ends** are white space, but a directive ends at one: each token says where the first
//! `\n` before it is (outside comments and `\`-continued lines, `Tok::nl`).
//!
//! Every word is a `Name`, with its word code (`w::…`; 0: none) from one hash probe: tree-sitter
//! reads a keyword only where its parse state takes it (keyword extraction), so the parser decides.
//! A string or character literal's prefix (`L`, `u`, `U`, `u8`, and C++'s `R`) is read with its
//! quote.
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
    Number,
    /// A character literal, its prefix included (its pieces are read by the parser).
    Char,
    /// A string literal, `"` to `"`, its prefix included.
    Str,
    /// A raw string literal (C++), `R"d(` to `)d"`, its prefix included.
    RawStr,
    /// A directive: `#`, blanks, and a word (its code), or `#` alone.
    Directive,
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Semi,
    Comma,
    Dot,
    DotStar,
    DotDotDot,
    Colon,
    ColonColon,
    Question,
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
    Arrow,
    ArrowStar,
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
    Spaceship,
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
    /// A name's or a directive's word code (`w::…`; 0: none).
    pub word: u8,
    pub start: u32,
    pub end: u32,
    /// The first `\n` between the token before and this one, outside comments (`NO_NL`: none).
    pub nl: u32,
}

/// A comment (the only extra).
#[derive(Clone, Copy, Debug)]
pub struct Extra {
    pub start: u32,
    pub end: u32,
}

/// Word codes: the keywords and the words the grammars name.
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
        // Primitive types (`primitive_type`, one token): `BOOL..=CHAR64_T`.
        BOOL = b"bool", CHAR = b"char", INT = b"int", FLOAT = b"float", DOUBLE = b"double", VOID = b"void",
        SIZE_T = b"size_t", SSIZE_T = b"ssize_t", PTRDIFF_T = b"ptrdiff_t", INTPTR_T = b"intptr_t",
        UINTPTR_T = b"uintptr_t", CHARPTR_T = b"charptr_t", NULLPTR_T = b"nullptr_t", MAX_ALIGN_T = b"max_align_t",
        INT8_T = b"int8_t", INT16_T = b"int16_t", INT32_T = b"int32_t", INT64_T = b"int64_t", UINT8_T = b"uint8_t",
        UINT16_T = b"uint16_t", UINT32_T = b"uint32_t", UINT64_T = b"uint64_t", CHAR8_T = b"char8_t",
        CHAR16_T = b"char16_t", CHAR32_T = b"char32_t", CHAR64_T = b"char64_t",
        // `sized_type_specifier`'s words.
        SIGNED = b"signed", UNSIGNED = b"unsigned", LONG = b"long", SHORT = b"short",
        // Storage classes.
        EXTERN = b"extern", STATIC = b"static", AUTO = b"auto", REGISTER = b"register", INLINE = b"inline",
        U_INLINE = b"__inline", U_INLINE_U = b"__inline__", U_FORCEINLINE = b"__forceinline",
        THREAD_LOCAL = b"thread_local", U_THREAD = b"__thread",
        // Type qualifiers.
        CONST = b"const", CONSTEXPR = b"constexpr", VOLATILE = b"volatile", RESTRICT = b"restrict",
        U_RESTRICT_U = b"__restrict__", U_EXTENSION_U = b"__extension__", ATOMIC = b"_Atomic",
        NORETURN_ = b"_Noreturn", NORETURN = b"noreturn", NONNULL = b"_Nonnull", ALIGNAS = b"alignas",
        ALIGNAS_ = b"_Alignas", MUTABLE = b"mutable", CONSTINIT = b"constinit", CONSTEVAL = b"consteval",
        // Microsoft modifiers.
        U_DECLSPEC = b"__declspec", U_BASED = b"__based", U_CDECL = b"__cdecl", U_CLRCALL = b"__clrcall",
        U_STDCALL = b"__stdcall", U_FASTCALL = b"__fastcall", U_THISCALL = b"__thiscall",
        U_VECTORCALL = b"__vectorcall", U_RESTRICT = b"__restrict", U_UPTR = b"__uptr", U_SPTR = b"__sptr",
        UNALIGNED_ = b"_unaligned", U_UNALIGNED = b"__unaligned",
        // Statements and the rest of C.
        BREAK = b"break", CASE = b"case", CONTINUE = b"continue", DEFAULT = b"default", DO = b"do",
        ELSE = b"else", ENUM = b"enum", FOR = b"for", GOTO = b"goto", IF = b"if", RETURN = b"return",
        SIZEOF = b"sizeof", STRUCT = b"struct", SWITCH = b"switch", TYPEDEF = b"typedef", UNION = b"union",
        WHILE = b"while", U_ATTRIBUTE_U = b"__attribute__", U_ATTRIBUTE = b"__attribute",
        ASM = b"asm", U_ASM_U = b"__asm__", U_ASM = b"__asm", U_VOLATILE_U = b"__volatile__",
        U_TRY = b"__try", U_EXCEPT = b"__except", U_FINALLY = b"__finally", U_LEAVE = b"__leave",
        OFFSETOF = b"offsetof", U_ALIGNOF_U = b"__alignof__", U_ALIGNOF = b"__alignof", ALIGNOF_ = b"_alignof",
        ALIGNOF = b"alignof", ALIGNOF_C = b"_Alignof", GENERIC = b"_Generic",
        TRUE = b"true", FALSE = b"false", TRUE_C = b"TRUE", FALSE_C = b"FALSE", NULL = b"NULL", NULLPTR = b"nullptr",
        DEFINED = b"defined",
        // C++.
        CLASS = b"class", NAMESPACE = b"namespace", TEMPLATE = b"template", TYPENAME = b"typename",
        USING = b"using", VIRTUAL = b"virtual", PUBLIC = b"public", PRIVATE = b"private", PROTECTED = b"protected",
        FRIEND = b"friend", OPERATOR = b"operator", NEW = b"new", DELETE = b"delete", THIS = b"this",
        THROW = b"throw", TRY = b"try", CATCH = b"catch", NOEXCEPT = b"noexcept", DECLTYPE = b"decltype",
        EXPLICIT = b"explicit", FINAL = b"final", OVERRIDE = b"override", CONCEPT = b"concept",
        REQUIRES = b"requires", CO_AWAIT = b"co_await", CO_RETURN = b"co_return", CO_YIELD = b"co_yield",
        STATIC_ASSERT = b"static_assert", AND = b"and", OR = b"or", NOT = b"not", XOR = b"xor",
        BITAND = b"bitand", BITOR = b"bitor", COMPL = b"compl", AND_EQ = b"and_eq", OR_EQ = b"or_eq",
        XOR_EQ = b"xor_eq", NOT_EQ = b"not_eq",
        // Directives (after `#`).
        INCLUDE = b"include", DEFINE = b"define", IFDEF = b"ifdef", IFNDEF = b"ifndef", ELIF = b"elif",
        ELIFDEF = b"elifdef", ELIFNDEF = b"elifndef", ENDIF = b"endif",
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
/// end, a comment's `/`, a `\`-newline's `\`)...
const GAP: u8 = 1;
/// ...and what does not start an ASCII name with no literal's prefix (`L`, `u`, `U`, `R`), so
/// that a 0 bit is such a name's first byte.
const NAME: u8 = 2;

/// A byte's `GAP` and `NAME` bits, by one load.
static FIRST: [u8; 256] = {
    let mut t = [NAME; 256];
    let mut c = 0;
    while c < 256 {
        let b = c as u8;
        if matches!(b, b' ' | b'\t' | b'\x0b' | b'\x0c' | b'\r' | b'\n' | b'/' | b'\\') {
            t[c] |= GAP;
        }
        if (b.is_ascii_alphabetic() || b == b'_' || b == b'$') && !matches!(b, b'L' | b'u' | b'U' | b'R') {
            t[c] &= !NAME;
        }
        c += 1;
    }
    t
};

pub struct Lexer<'s> {
    pub src: &'s [u8],
    pub blocks: Blocks<'s>,
    /// C++'s lexer (raw strings, `::`, `.*`, `->*`, `<=>`).
    pub cpp: bool,
}

impl<'s> Lexer<'s> {
    pub fn new(src: &'s [u8], cpp: bool) -> Self {
        Lexer { src, blocks: Blocks::of(src), cpp }
    }

    #[inline(always)]
    pub fn at(&self, i: usize) -> u8 {
        self.src.get(i).copied().unwrap_or(0)
    }

    /// The end of a `\`-newline (`\\\r?\n`) at `i`, if one is there.
    #[inline(always)]
    fn continuation(&self, i: usize) -> Option<usize> {
        if self.at(i) != b'\\' {
            return None;
        }
        match self.at(i + 1) {
            b'\n' => Some(i + 2),
            b'\r' if self.at(i + 2) == b'\n' => Some(i + 3),
            _ => None,
        }
    }

    /// From `at`: white space and comments passed (the comments pushed to `extras`); where the next
    /// token starts, and the first `\n` passed outside comments (`NO_NL`: none).
    #[inline]
    pub fn skip(&self, mut at: usize, extras: &mut Vec<Extra>) -> R<(usize, u32)> {
        // Most tokens follow the one before at once (`f(`, `a;`): no blank, line end, comment or
        // `\`-newline to pass. Past the text's end the byte read is 0, which starts none.
        if FIRST[usize::from(self.at(at))] & GAP == 0 {
            return Ok((at, NO_NL));
        }
        let mut nl = NO_NL;
        loop {
            at = self.blocks.blank_end(at);
            match self.at(at) {
                b'\n' if at < self.src.len() => {
                    if nl == NO_NL {
                        nl = at as u32;
                    }
                    at += 1;
                }
                b'\\' => match self.continuation(at) {
                    Some(e) => at = e,
                    None => return Ok((at, nl)),
                },
                b'/' => {
                    let Some(x) = self.comment(at)? else { return Ok((at, nl)) };
                    extras.push(x);
                    at = x.end as usize;
                }
                _ => return Ok((at, nl)),
            }
        }
    }

    /// The comment starting at `at` (its `/`), if one does; an unclosed `/*` refused. A line comment
    /// goes on past a `\` before its line's end.
    #[inline]
    pub fn comment(&self, at: usize) -> R<Option<Extra>> {
        match self.at(at + 1) {
            b'/' => {
                let mut i = at + 2;
                loop {
                    i = self.blocks.eol(i);
                    if i >= self.src.len() {
                        break;
                    }
                    // `(\\+(.|\r?\n)|[^\\\n])*`: a `\n` after a run of `\` (and a `\r`) is the comment's.
                    let mut k = i;
                    if k > at + 2 && self.src[k - 1] == b'\r' {
                        k -= 1;
                    }
                    let mut slashes = 0;
                    while k > at + 2 + slashes && self.src[k - 1 - slashes] == b'\\' {
                        slashes += 1;
                    }
                    if slashes == 0 {
                        break;
                    }
                    i += 1;
                }
                Ok(Some(Extra { start: at as u32, end: i as u32 }))
            }
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
        let (s, nl) = self.skip(at, extras)?;
        let (kind, end, word) = self.token_at(s)?;
        Ok(Tok { kind, word, start: s as u32, end: end as u32, nl })
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
        // An ASCII name that is no literal's prefix (`L"…"`, `u8'…'`, `R"(…)"`): its end is where its
        // word bytes stop, unless a `\u` escape or a character past ASCII goes on there.
        if FIRST[usize::from(c)] & NAME == 0 {
            let end = self.blocks.word_end(s + 1);
            let d = self.at(end);
            if d != b'\\' && d < 0x80 {
                return Ok((Name, end, word(self.src, s, end)));
            }
            let end = self.name_end(s)?;
            return Ok((Name, end, word(self.src, s, end)));
        }
        let (kind, len) = match c {
            b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'$' => {
                // A literal's prefix: `L`, `u`, `U`, `u8`, and C++'s `R`, `LR`, `uR`, `UR`, `u8R`.
                if matches!(c, b'L' | b'u' | b'U' | b'R')
                    && let Some(t) = self.prefixed(s) {
                        return t;
                    }
                let end = self.name_end(s)?;
                return Ok((Name, end, word(self.src, s, end)));
            }
            b'0'..=b'9' => return self.number(s),
            b'.' if next.is_ascii_digit() => return self.number(s),
            b'"' => {
                return match self.string_end(s + 1) {
                    Some(end) => Ok((Str, end, 0)),
                    None => refuse,
                };
            }
            b'\'' => match self.char_end(s + 1) {
                Some(end) => return Ok((Char, end, 0)),
                None => return refuse,
            },
            b'#' => {
                let mut k = s + 1;
                while matches!(self.at(k), b' ' | b'\t') {
                    k += 1;
                }
                let e = self.blocks.word_end(k);
                if e == k {
                    return Ok((Directive, s + 1, 0));
                }
                return Ok((Directive, e, word(self.src, k, e)));
            }
            b'(' => (LParen, 1),
            b')' => (RParen, 1),
            b'[' => (LBracket, 1),
            b']' => (RBracket, 1),
            b'{' => (LBrace, 1),
            b'}' => (RBrace, 1),
            b';' => (Semi, 1),
            b',' => (Comma, 1),
            b'~' => (Tilde, 1),
            b'?' => (Question, 1),
            b':' => match next {
                b':' if self.cpp => (ColonColon, 2),
                _ => (Colon, 1),
            },
            b'.' => {
                if next == b'.' && self.at(s + 2) == b'.' {
                    (DotDotDot, 3)
                } else if next == b'*' && self.cpp {
                    (DotStar, 2)
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
            b'-' => match (next, self.at(s + 2)) {
                (b'-', _) => (MinusMinus, 2),
                (b'=', _) => (MinusEq, 2),
                (b'>', b'*') if self.cpp => (ArrowStar, 3),
                (b'>', _) => (Arrow, 2),
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
                (b'=', b'>') if self.cpp => (Spaceship, 3),
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

    /// A string, character or raw string literal at `s` whose prefix is a name's start: its token,
    /// or `None` where `s` starts a name.
    fn prefixed(&self, s: usize) -> Option<R<(T, usize, u8)>> {
        let mut k = s;
        match self.at(k) {
            b'L' | b'U' => k += 1,
            b'u' => k += if self.at(k + 1) == b'8' { 2 } else { 1 },
            _ => {}
        }
        let raw = self.cpp && self.at(k) == b'R';
        if raw {
            k += 1;
        }
        let q = self.at(k);
        if k == s || !(q == b'"' || q == b'\'' && !raw) {
            return None;
        }
        let refuse = Err(Refusal::Lex(s as u32));
        Some(if raw {
            self.raw_end(k + 1).map_or(refuse, |e| Ok((T::RawStr, e, 0)))
        } else if q == b'"' {
            self.string_end(k + 1).map_or(refuse, |e| Ok((T::Str, e, 0)))
        } else {
            self.char_end(k + 1).map_or(refuse, |e| Ok((T::Char, e, 0)))
        })
    }

    /// The end of a raw string whose `"` ends at `i`: `d(` … `)d"`.
    fn raw_end(&self, i: usize) -> Option<usize> {
        let mut k = i;
        while k < self.src.len() && !matches!(self.src[k], b'(' | b')' | b'\\' | b' ' | b'\t' | b'\n' | b'"') && k - i <= 16 {
            k += 1;
        }
        if self.at(k) != b'(' {
            return None;
        }
        let delim = &self.src[i..k];
        let mut j = k + 1;
        loop {
            let close = j + self.src.get(j..)?.iter().position(|&c| c == b')')?;
            let after = close + 1;
            if self.src.get(after..after + delim.len()) == Some(delim) && self.at(after + delim.len()) == b'"' {
                return Some(after + delim.len() + 1);
            }
            j = close + 1;
        }
    }

    /// The end of a name from `s` (`[_$\p{XID_Start}][_$\p{XID_Continue}]*`; `\u` escapes refused):
    /// ASCII word bytes a block a step, a character past ASCII by its class. `s` itself where no
    /// name starts there.
    pub fn name_end(&self, s: usize) -> R<usize> {
        let c = self.at(s);
        if c.is_ascii_digit() || (c < 0x80 && !block::is_word(c)) {
            return Ok(s);
        }
        let mut i = s;
        loop {
            i = self.blocks.word_end(i);
            if self.at(i) == b'\\' && matches!(self.at(i + 1), b'u' | b'U') {
                return Err(Refusal::Lex(i as u32));
            }
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

    /// `digit+ ('digit+)*` from `i`: its end, `None` where no digit starts it.
    fn digits(&self, mut i: usize, digit: impl Fn(u8) -> bool) -> Option<usize> {
        if !digit(self.at(i)) {
            return None;
        }
        loop {
            i += 1;
            while digit(self.at(i)) {
                i += 1;
            }
            if self.at(i) == b'\'' && digit(self.at(i + 1)) {
                i += 1;
                continue;
            }
            return Some(i);
        }
    }

    /// A number at `s` (a digit, or a `.` before one): tree-sitter-c's `number_literal`, whose
    /// mantissa after a `.` and whose exponent take hex digits, and whose suffix is any run of
    /// `[uUlLwWfFbBdD]`. C++'s reads the same here; its suffixes are checked by the parser.
    fn number(&self, s: usize) -> R<(T, usize, u8)> {
        let hexd = |c: u8| c.is_ascii_hexdigit();
        let dec = |c: u8| c.is_ascii_digit();
        let (c0, c1) = (self.at(s), self.at(s + 1));
        // C++: `0x` with no digit after is `0` and a user-defined literal's suffix `x`
        // (tree-sitter-cpp's longest match).
        let mut i = if c0 == b'0' && matches!(c1, b'x' | b'X') {
            self.digits(s + 2, hexd).unwrap_or(if self.cpp && self.at(s + 2) != b'.' { s + 1 } else { s + 2 })
        } else if c0 == b'0' && matches!(c1, b'b' | b'B') {
            self.digits(s + 2, dec).unwrap_or(if self.cpp { s + 1 } else { s + 2 })
        } else if c0 == b'.' {
            s
        } else {
            self.digits(s, dec).unwrap_or(s)
        };
        if self.at(i) == b'.' {
            i = self.digits(i + 1, hexd).unwrap_or(i + 1);
        }
        // The longest match: a hex run's last `e`/`p` is the exponent's where a sign and a digit
        // follow it (`1.5E-3`, `0x1e+5`).
        if matches!(self.at(i), b'+' | b'-') && i > s && matches!(self.src[i - 1], b'e' | b'E' | b'p' | b'P') && hexd(self.at(i + 1)) {
            i = self.digits(i + 1, hexd).unwrap_or(i + 1);
        } else if matches!(self.at(i), b'e' | b'E' | b'p' | b'P') {
            let k = if matches!(self.at(i + 1), b'+' | b'-') { i + 2 } else { i + 1 };
            i = self.digits(k, hexd).unwrap_or(if k == i + 2 { i + 1 } else { k });
        }
        while matches!(self.at(i), b'u' | b'U' | b'l' | b'L' | b'w' | b'W' | b'f' | b'F' | b'b' | b'B' | b'd' | b'D') {
            i += 1;
        }
        // A name's byte right after a number is no token tree-sitter reads apart (in C++, a
        // user-defined literal's suffix, which the parser does not read yet).
        if !self.cpp && (block::is_word(self.at(i)) || self.at(i) >= 0x80) {
            return Err(Refusal::Lex(i as u32));
        }
        Ok((T::Number, i, 0))
    }

    /// The end of a string's text from `i` (past its opening `"`): past its closer. `None` where a
    /// line or the file ends first.
    pub fn string_end(&self, mut i: usize) -> Option<usize> {
        loop {
            // The next `"`, `\` or `\n` (stage 1's mask): the text between is passed a block at a time.
            i = self.blocks.string_stop(i);
            match self.src.get(i)? {
                b'"' => return Some(i + 1),
                b'\n' => return None,
                _ => {
                    // An escape: `\` and its character (a `\r\n` after `\` is one).
                    i = self.escape_end(i)?;
                }
            }
        }
    }

    /// A character literal's end from `i` (past its `'`): characters but `\n` and `'`, or escapes;
    /// at least one; then `'`.
    fn char_end(&self, mut i: usize) -> Option<usize> {
        let start = i;
        loop {
            match self.at(i) {
                b'\'' if i > start => return Some(i + 1),
                b'\'' | b'\n' => return None,
                b'\\' => i = self.escape_end(i)?,
                _ if i >= self.src.len() => return None,
                _ => i += 1,
            }
        }
    }

    /// An escape sequence from `i` (its `\`), as tree-sitter-c's `escape_sequence` reads one: the
    /// longest of `\` and one character but `x`, `u` and `U`; 2 or 3 digits; `x` and 1 to 4 hex
    /// digits; `u` and 4; `U` and 8. Its end.
    pub fn escape_end(&self, i: usize) -> Option<usize> {
        let at = |k: usize| self.at(k);
        let hex = |from: usize, n: usize| (from..from + n).all(|k| at(k).is_ascii_hexdigit());
        if i + 1 >= self.src.len() {
            return None;
        }
        match at(i + 1) {
            b'x' => {
                let mut k = i + 2;
                while k < i + 6 && at(k).is_ascii_hexdigit() {
                    k += 1;
                }
                (k > i + 2).then_some(k)
            }
            b'u' => hex(i + 2, 4).then_some(i + 6),
            b'U' => hex(i + 2, 8).then_some(i + 10),
            b'0'..=b'9' => {
                let mut k = i + 1;
                while k < i + 4 && at(k).is_ascii_digit() {
                    k += 1;
                }
                Some(if k >= i + 3 { k } else { i + 2 })
            }
            b'\r' if at(i + 2) == b'\n' => Some(i + 2),
            // A line continuation then a one-character escape: one escape (tree-sitter's lexer
            // reads `\<newline>\n` as one token).
            b'\n' if at(i + 2) == b'\\' && !matches!(at(i + 3), b'\n' | b'\r' | b'x' | b'u' | b'U' | b'0'..=b'9') && at(i + 3) < 0x80 => Some(i + 4),
            c if c >= 0x80 => Some(i + 1 + decode(self.src, i + 1)?.1),
            _ => Some(i + 2),
        }
    }

    /// A directive's argument from `at` (`preproc_arg`: `\S([^/\n]|\/[^*]|\\\r?\n)*`), blanks and
    /// `\`-newlines before it passed: its start and end; `None` where the line (or the file) ends
    /// first, or a comment starts. `Err` where tree-sitter's read is not one this parser keeps.
    ///
    /// The line end after a directive is `token.immediate`: after a blank it cannot be read, so
    /// white space (line ends too) is passed and the next line's text is the argument
    /// (`#define A \nint x;` defines `A` as `int x;`).
    pub fn arg(&self, mut at: usize) -> Result<Option<(usize, usize)>, Refusal> {
        let from = at;
        loop {
            at = self.blocks.blank_end(at);
            match self.continuation(at) {
                Some(e) => at = e,
                None => break,
            }
        }
        // (A `\r` just before the `\n` is the line end's: `\r?\n`.)
        let blanks_end = if at > from && self.at(at - 1) == b'\r' { at - 1 } else { at };
        if blanks_end > from && self.at(at) == b'\n' {
            while at < self.src.len() && (self.at(at).is_ascii_whitespace() || self.at(at) == 0x0b) {
                at = self.continuation(at).unwrap_or(at + 1);
            }
            // The file's end, or a comment (an extra, after which the line end is no immediate
            // token either): tree-sitter's reading there is not kept.
            if at >= self.src.len() || self.at(at) == b'/' && matches!(self.at(at + 1), b'*' | b'/') {
                return Err(Refusal::Parse(at as u32));
            }
        }
        match self.at(at) {
            b'\n' | 0 if at >= self.src.len() || self.at(at) == b'\n' => return Ok(None),
            b'/' if matches!(self.at(at + 1), b'*' | b'/') => return Ok(None),
            _ => {}
        }
        let s = at;
        let mut i = at + 1;
        loop {
            i = self.blocks.arg_stop(i);
            if i >= self.src.len() {
                return Ok(Some((s, self.src.len())));
            }
            match self.src[i] {
                b'\n' => return Ok(Some((s, i))),
                b'\\' => i += if self.at(i + 1) == b'\r' && self.at(i + 2) == b'\n' { 3 } else { 2 },
                _ => {
                    // `/`: the argument's unless a block comment starts.
                    if self.at(i + 1) == b'*' {
                        return Ok(Some((s, i)));
                    }
                    i += 2;
                }
            }
        }
    }

    /// `<…>` from `s` (an `#include`'s `system_lib_string`: `<`, `[^>\n]` or `\>`, `>`): its end.
    pub fn system_lib(&self, s: usize) -> Option<usize> {
        let mut i = s + 1;
        loop {
            match self.at(i) {
                b'>' => return Some(i + 1),
                b'\\' if self.at(i + 1) == b'>' => i += 2,
                b'\n' => return None,
                _ if i >= self.src.len() => return None,
                _ => i += 1,
            }
        }
    }
}
