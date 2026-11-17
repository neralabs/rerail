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

/// The token a refusal is read as where the parser comes to it: an `Eof` with word `u8::MAX`, at the
/// refused byte (at 0 for a text too large).
#[cold]
pub fn refused(r: Refusal) -> Tok {
    let at = match r {
        Refusal::Lex(at) | Refusal::Parse(at) => at,
        Refusal::TooLarge => 0,
    };
    Tok { kind: T::Eof, word: u8::MAX, start: at, end: at, nl: NO_NL }
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

// The run's name path is timed on the direct shape: one multiply, no displacement.
const _: () = assert!(WORDS.is_direct(), "the keywords' switch has the direct shape");

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
/// ...a blank that may be a gap of one (`' '`, `'\t'`)...
const BLANK1: u8 = 4;
/// ...and a gap's byte that `token` reads (a comment's `/` or a `\\`).
const SLASH: u8 = 8;

/// A byte's `GAP`, `NAME`, `BLANK1` and `SLASH` bits, by one load.
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
        if matches!(b, b' ' | b'\t') {
            t[c] |= BLANK1;
        }
        if matches!(b, b'/' | b'\\') {
            t[c] |= SLASH;
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
    t[b'?' as usize] = T::Question;
    t
};

/// `LEAD`'s classes of a token's first byte in `run`: a one-byte punctuator (`ONE`; its kind in
/// `one`)...
const L_ONE: u8 = 0;
/// ...an ASCII name with no literal's prefix (`FIRST`'s `NAME` clear)...
const L_NAME: u8 = 1;
/// ...a byte read from the gap's start by `token` (a comment's `/`, a `\\`, a NUL, or the text's end, read as a 0)...
const L_TOKEN: u8 = 2;
/// ...or the rest (`decimal`, then `token_at`).
const L_REST: u8 = 3;

#[derive(Clone, Copy)]
struct Lead {
    one: T,
    class: u8,
}

/// A token's first byte's class and one-byte kind, by one load.
static LEAD: [Lead; 256] = {
    let mut t = [Lead { one: T::Eof, class: L_REST }; 256];
    let mut c = 0;
    while c < 256 {
        if FIRST[c] & NAME == 0 {
            t[c].class = L_NAME;
        }
        if ONE[c] as u8 != T::Eof as u8 {
            t[c] = Lead { one: ONE[c], class: L_ONE };
        }
        c += 1;
    }
    t[0].class = L_TOKEN;
    t[b'/' as usize].class = L_TOKEN;
    t[b'\\' as usize].class = L_TOKEN;
    t
};

/// `t` written at `p`, a field at a time: its padding is left unwritten (a write of the whole
/// token copies it, from wherever the token was made).
///
/// # Safety
/// `p` is valid for a write of a `Tok`.
#[inline(always)]
unsafe fn put(p: *mut std::mem::MaybeUninit<Tok>, t: Tok) {
    let p = p.cast::<Tok>();
    // SAFETY: as above; each field's place is in the token's room.
    unsafe {
        (&raw mut (*p).kind).write(t.kind);
        (&raw mut (*p).word).write(t.word);
        (&raw mut (*p).start).write(t.start);
        (&raw mut (*p).end).write(t.end);
        (&raw mut (*p).nl).write(t.nl);
    }
}

pub struct Lexer<'s> {
    pub src: &'s [u8],
    pub blocks: Blocks,
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
            // To the first `*/` from the comment's third byte (`/*/` is open), one micro-lexer scan.
            b'*' => match rerail_simd8::micro::find_pair(self.src, at + 2, b'*', b'/') {
                Some(i) => Ok(Some(Extra { start: at as u32, end: i as u32 + 2 })),
                None => Err(Refusal::Lex(at as u32)),
            },
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

    /// Up to `n` tokens from `at` pushed onto `out`, comments onto `extras`: where the next is
    /// lexed. A run ends early after `Eof`, or after a refusal, written as `refused` writes it.
    ///
    /// simdjson's stage 2 in page 31's form: each token's start is the next one of stage 1's start
    /// masks (`Blocks::next_start`), the line end before it a scan of the gap, and an ASCII name's
    /// end its word run, all from the masks. A token that is not a name is read by `token_at` from
    /// its start; a comment or a `\` in the gap sends the token to `token` from the gap's start.
    #[inline(always)]
    pub fn run(&self, mut at: usize, extras: &mut Vec<Extra>, out: &mut Vec<Tok>, n: usize) -> usize {
        let mut cur = rerail_simd8::blocks::Cursor::default();
        out.reserve(n);
        let base = out.len();
        // simdjson's tape writer: tokens written into the room reserved, the length set once.
        let w = out.spare_capacity_mut().as_mut_ptr();
        let mut k = 0;
        while k < n {
            let c0 = self.at(at);
            let f0 = FIRST[usize::from(c0)];
            let (s, c, nl) = if f0 & GAP == 0 {
                // No gap: a token right here, though it need be no start (after a number like
                // C++'s `0x`, a name goes on inside a word run).
                (at, c0, NO_NL)
            } else if f0 & SLASH != 0 {
                // A comment or a `\\`-newline in the gap: `token` reads from the gap's start (a 0
                // byte's class).
                (at, 0, NO_NL)
            } else if f0 & BLANK1 != 0 && FIRST[usize::from(self.at(at + 1))] & GAP == 0 {
                // One blank, the most common gap: no walk and no line end to look for.
                (at + 1, self.at(at + 1), NO_NL)
            } else if let Some((s, eols)) = self.blocks.gap(at) {
                // A gap that ends in its block: its end and its line ends by one run of the masks.
                (s, self.at(s), if eols != 0 { (at & !63) as u32 + eols.trailing_zeros() } else { NO_NL })
            } else {
                // A gap on past its block: the next start ends it.
                let s = self.blocks.next_start(&mut cur, at);
                (s, self.at(s), self.blocks.eol_in(at, s).map_or(NO_NL, |e| e as u32))
            };
            // The start byte's class by one load (the text's end read as a 0, as a NUL).
            let lead = LEAD[usize::from(c)];
            let t = if lead.class == L_ONE {
                Tok { kind: lead.one, word: 0, start: s as u32, end: s as u32 + 1, nl }
            } else if lead.class == L_NAME {
                let end = self.blocks.word_end(s + 1);
                let d = self.at(end);
                if d != b'\\' && d < 0x80 {
                    Tok { kind: T::Name, word: word(self.src, s, end), start: s as u32, end: end as u32, nl }
                } else {
                    self.token(at, extras).unwrap_or_else(refused)
                }
            } else if lead.class == L_TOKEN {
                self.token(at, extras).unwrap_or_else(refused)
            } else if let Some(end) = self.decimal(s) {
                Tok { kind: T::Number, word: 0, start: s as u32, end: end as u32, nl }
            } else {
                match self.token_at(s) {
                    Ok((kind, end, word)) => Tok { kind, word, start: s as u32, end: end as u32, nl },
                    Err(r) => refused(r),
                }
            };
            // SAFETY: `k < n`, and `n` tokens' room is reserved past `base`.
            unsafe { put(w.add(k), t) };
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

    /// A decimal integer of up to 8 digits at `s` with no suffix, fraction, exponent or `'`, no
    /// character past ASCII after it (C refuses one), and no leading `0` (C++'s `0x` and `0b` take a suffix's place): its end, from its word run and
    /// one test of its 8 bytes. `number` reads every other number.
    #[inline(always)]
    fn decimal(&self, s: usize) -> Option<usize> {
        let c = self.src[s];
        if !(b'1'..=b'9').contains(&c) {
            return None;
        }
        let end = self.blocks.word_end(s + 1);
        let n = end - s;
        let d = self.at(end);
        if n > 8 || d == b'.' || d == b'\'' || d >= 0x80 || !rerail_simd8::swar::digits(rerail_simd8::swar::load8(self.src, s), n) {
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
            _ => return self.token_rare(s),
        };
        Ok((kind, s + len, 0))
    }

    /// `token_at`'s tokens that are no punctuator: names, numbers, literals, directives (and a
    /// refusal), outlined so that punctuation is read with no frame to save.
    #[inline(never)]
    fn token_rare(&self, s: usize) -> R<(T, usize, u8)> {
        use T::*;
        let c = self.src[s];
        let refuse = Err(Refusal::Lex(s as u32));
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
        match c {
            b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'$' => {
                // A literal's prefix: `L`, `u`, `U`, `u8`, and C++'s `R`, `LR`, `uR`, `UR`, `u8R`.
                if matches!(c, b'L' | b'u' | b'U' | b'R') {
                    if let Some(t) = self.prefixed(s) {
                        return t;
                    }
                }
                let end = self.name_end(s)?;
                return Ok((Name, end, word(self.src, s, end)));
            }
            b'0'..=b'9' => return self.number(s),
            b'.' => self.number(s),
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
            let close = rerail_simd8::micro::find(self.src, j, b')')?;
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
            i = rerail_simd8::micro::first_of(self.src, i, [b'"', b'\\', b'\n']);
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
            i = rerail_simd8::micro::first_of(self.src, i, [b'/', b'\\', b'\n']);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The tokens of `src` one `token` at a time (the scalar lexer), and the comments.
    fn by_token(lx: &Lexer, mut at: usize) -> (Vec<(T, u8, u32, u32, u32)>, Vec<(u32, u32)>) {
        let (mut out, mut extras) = (Vec::new(), Vec::new());
        loop {
            let t = lx.token(at, &mut extras).unwrap_or_else(refused);
            out.push((t.kind, t.word, t.start, t.end, t.nl));
            if t.kind == T::Eof {
                break;
            }
            at = t.end as usize;
        }
        (out, extras.iter().map(|x| (x.start, x.end)).collect())
    }

    /// The same by `run`, in batches of `n`.
    fn by_run(lx: &Lexer, mut at: usize, n: usize) -> (Vec<(T, u8, u32, u32, u32)>, Vec<(u32, u32)>) {
        let (mut toks, mut extras) = (Vec::new(), Vec::new());
        loop {
            at = lx.run(at, &mut extras, &mut toks, n);
            if toks.last().is_some_and(|t| t.kind == T::Eof) {
                break;
            }
        }
        (toks.iter().map(|t| (t.kind, t.word, t.start, t.end, t.nl)).collect(), extras.iter().map(|x| (x.start, x.end)).collect())
    }

    /// `run` reads what `token` reads: every token, comment and refusal, over random texts of
    /// C's pieces (and from every position of a few, as a relex starts anywhere).
    #[test]
    fn run_reads_as_token_does() {
        const PIECES: &[&str] = &[
            "a", "int", "x1", "_y", "L", "u8", "R", "0x", "0", "7", "42", "12345678", "123456789", "1.5", "1e5", "3'4",
            "10u", " ", "  ", "\t", "\n", "\r\n", "\\\n", "\\", "/", "//c\n", "/*c*/", "/*\n*/", "(", ")", "{", ";", ",",
            "?", "~", ":", "::", ".", "...", "->", "->*", "<", "<<=", "<=>", ">>", "=", "==", "!", "&&", "|", "^=", "%",
            "\"s\"", "\"a\\\"b\"", "'c'", "'\\n'", "L\"w\"", "R\"(r)\"", "#", "# include", "#define", "é", "\u{ff}",
        ];
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        for cpp in [false, true] {
            for round in 0..1500 {
                let mut src = String::new();
                for _ in 0..(round % 90) {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    src += PIECES[(x >> 20) as usize % PIECES.len()];
                }
                let lx = Lexer::new(src.as_bytes(), cpp);
                let starts: Vec<usize> = if round % 50 == 0 { (0..=src.len()).collect() } else { vec![0] };
                for at in starts {
                    let want = by_token(&lx, at);
                    for n in [1, 3, 16] {
                        assert_eq!(by_run(&lx, at, n), want, "{src:?} from {at}, cpp {cpp}, batches of {n}");
                    }
                }
            }
        }
    }
}
