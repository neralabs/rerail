//! The lexer: tree-sitter-c-sharp's lexer, read where the parser asks. The lexer keeps no state:
//! the parser asks for the next token at a byte (`Lexer::token`), and reads itself the few places
//! its parse state lexes otherwise (a `>>` split in two in type arguments, a string's pieces).
//!
//! Every word is a `Name`, with its word code (`w::…`; 0: none) from one hash probe: tree-sitter
//! reads a keyword only where its parse state takes it (keyword extraction), so the parser decides.
//!
//! A string of any form (regular, verbatim, raw, interpolated) is one token over the whole literal,
//! as tree-sitter-c-sharp's external scanner reads it; the parser reads its pieces from the bytes.
//!
//! Comments and the preprocessor's lines (but `#if`, `#elif`, `#else`, `#endif`, which are the
//! grammar's tokens) are extras, in a list of their own, written where the parser next opens a
//! node or reads a token.

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
    /// `integer_literal`.
    Int,
    /// `real_literal`.
    Real,
    /// `character_literal`, `'` to `'`.
    Char,
    /// A `string_literal`, `"` to `"` (and its `u8`).
    Str,
    /// A `verbatim_string_literal`, `@"` to `"`.
    Verbatim,
    /// A `raw_string_literal`, `"""` to `"""`.
    Raw,
    /// An `interpolated_string_expression`, `$"` (`$@"`, `@$"`, `$$"""`…) to its closer.
    Interp,
    /// `#if` and its condition's line (the condition is lexed as tokens after it).
    PpIf,
    PpElif,
    PpElse,
    PpEndif,
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
    ColonColon,
    Colon,
    Question,
    QQ,
    QQEq,
    FatArrow,
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

/// An extra's kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum X {
    Comment,
    Region,
    Endregion,
    Pragma,
    Nullable,
    Define,
    Undef,
    Error,
    Warning,
    Line,
}

/// An extra: a comment, or a preprocessor line (to its `\n`, which it holds).
#[derive(Clone, Copy, Debug)]
pub struct Extra {
    pub kind: X,
    pub start: u32,
    pub end: u32,
    /// A directive's word's end (`#region` …), past which its arguments start.
    pub word_end: u32,
}

/// Word codes: the keywords, the contextual words and the predefined types.
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
        // C#'s reserved keywords (`ABSTRACT..=WHILE`), never an identifier.
        ABSTRACT = b"abstract", AS = b"as", BASE = b"base", BREAK = b"break", CASE = b"case",
        CATCH = b"catch", CHECKED = b"checked", CLASS = b"class", CONST = b"const", CONTINUE = b"continue",
        DEFAULT = b"default", DELEGATE = b"delegate", DO = b"do", ELSE = b"else", ENUM = b"enum",
        EVENT = b"event", EXPLICIT = b"explicit", EXTERN = b"extern", FALSE = b"false", FINALLY = b"finally",
        FIXED = b"fixed", FOR = b"for", FOREACH = b"foreach", GOTO = b"goto", IF = b"if",
        IMPLICIT = b"implicit", IN = b"in", INTERFACE = b"interface", INTERNAL = b"internal", IS = b"is",
        LOCK = b"lock", NAMESPACE = b"namespace", NEW = b"new", NULL = b"null", OPERATOR = b"operator",
        OUT = b"out", OVERRIDE = b"override", PARAMS = b"params", PRIVATE = b"private", PROTECTED = b"protected",
        PUBLIC = b"public", READONLY = b"readonly", REF = b"ref", RETURN = b"return", SEALED = b"sealed",
        SIZEOF = b"sizeof", STACKALLOC = b"stackalloc", STATIC = b"static", STRUCT = b"struct",
        SWITCH = b"switch", THIS = b"this", THROW = b"throw", TRUE = b"true", TRY = b"try", TYPEOF = b"typeof",
        UNCHECKED = b"unchecked", UNSAFE = b"unsafe", USING = b"using", VIRTUAL = b"virtual",
        VOLATILE = b"volatile", WHILE = b"while",
        // The predefined types (`BOOL..=VOID`).
        BOOL = b"bool", BYTE = b"byte", CHAR = b"char", DECIMAL = b"decimal", DOUBLE = b"double",
        FLOAT = b"float", INT = b"int", LONG = b"long", OBJECT = b"object", SBYTE = b"sbyte", SHORT = b"short",
        STRING = b"string", UINT = b"uint", ULONG = b"ulong", USHORT = b"ushort", NINT = b"nint",
        NUINT = b"nuint", VOID = b"void",
        // The reserved identifiers (`ALIAS..=YIELD`): an `identifier` holding the word's token.
        ALIAS = b"alias", ASCENDING = b"ascending", BY = b"by", DESCENDING = b"descending",
        EQUALS = b"equals", FILE = b"file", FROM = b"from", GLOBAL = b"global", GROUP = b"group",
        INTO = b"into", JOIN = b"join", LET = b"let", NOTNULL = b"notnull", ON = b"on", ORDERBY = b"orderby",
        SCOPED = b"scoped", SELECT = b"select", UNMANAGED = b"unmanaged", VAR = b"var", WHEN = b"when",
        WHERE = b"where", YIELD = b"yield",
        // The other contextual words: names where their keyword is not read.
        ADD = b"add", AND = b"and", ASSEMBLY = b"assembly", ASYNC = b"async", AWAIT = b"await",
        FIELD = b"field", GET = b"get", INIT = b"init", MANAGED = b"managed",
        METHOD = b"method", MODULE = b"module", NOT = b"not", OR = b"or", PARAM = b"param",
        PARTIAL = b"partial", PROPERTY = b"property", RECORD = b"record", REMOVE = b"remove",
        REQUIRED = b"required", SET = b"set", TYPE = b"type", TYPEVAR = b"typevar", WITH = b"with",
        UNDERSCORE = b"_", MAKEREF = b"__makeref", REFTYPE = b"__reftype", REFVALUE = b"__refvalue",
        CDECL = b"Cdecl", STDCALL = b"Stdcall", THISCALL = b"Thiscall", FASTCALL = b"Fastcall",
        // The preprocessor's words.
        WARNING = b"warning", DISABLE = b"disable", RESTORE = b"restore", CHECKSUM = b"checksum",
        ENABLE = b"enable", ANNOTATIONS = b"annotations", WARNINGS = b"warnings", HIDDEN = b"hidden",
    }

    /// Whether `code` is a reserved keyword's (never an identifier).
    pub const fn is_reserved(code: u8) -> bool {
        code >= ABSTRACT && code <= VOID
    }

    /// Whether `code` is a predefined type's word (`void` included).
    pub const fn is_predefined(code: u8) -> bool {
        code >= BOOL && code <= VOID
    }

    /// Whether `code` is a reserved identifier's (`_reserved_identifier`).
    pub const fn is_reserved_identifier(code: u8) -> bool {
        code >= ALIAS && code <= YIELD
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

/// The word `src[i..end]`'s code: its first 8 bytes loaded as one integer, masked to its length,
/// hashed to its one slot and compared once (and its tail, for the few words past 8 bytes).
#[inline(always)]
pub fn word(src: &[u8], i: usize, end: usize) -> u8 {
    let len = end - i;
    if len.wrapping_sub(1) >= 16 {
        return 0;
    }
    let raw = match src.get(i..i + 8) {
        Some(b) => u64::from_le_bytes(b.try_into().expect("8 bytes")),
        None => word_key(&src[i..end]),
    };
    let k = if len >= 8 { raw } else { raw & (u64::MAX >> (64 - 8 * len)) };
    let code = SLOTS[word_slot(k, len)];
    let (wk, wl) = KEYS[usize::from(code)];
    if code == 0 || wk != k || usize::from(wl) != len {
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

    /// Whether only blanks stand between the line's start and `at`.
    fn line_start(&self, at: usize) -> bool {
        let mut i = at;
        while i > 0 {
            // The file's byte order mark is white space.
            if i == 3 && self.src.starts_with(b"\xef\xbb\xbf") {
                return true;
            }
            match self.src[i - 1] {
                b' ' | b'\t' | 0x0b | 0x0c => i -= 1,
                b'\n' | b'\r' => return true,
                _ => return false,
            }
        }
        true
    }

    /// From `at`: white space, comments and directive lines passed (pushed to `extras`); where the
    /// next token starts.
    #[inline]
    pub fn skip(&self, mut at: usize, extras: &mut Vec<Extra>) -> R<usize> {
        loop {
            at = self.blocks.blank_end(at);
            match self.at(at) {
                b'\n' | b'\r' => at += 1,
                b'/' if self.at(at + 1) == b'/' => {
                    // `//` and what follows but a line end (`[^\n\r]*`).
                    let end = self.blocks.eol(at + 2);
                    extras.push(Extra { kind: X::Comment, start: at as u32, end: end as u32, word_end: 0 });
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
                    extras.push(Extra { kind: X::Comment, start: at as u32, end: i as u32, word_end: 0 });
                    at = i;
                }
                b'#' if at < self.src.len() => match self.directive(at)? {
                    Some(x) => {
                        at = x.end as usize;
                        extras.push(x);
                    }
                    None => return Ok(at),
                },
                c if c >= 0x80 => match decode(self.src, at) {
                    Some((c, n)) if c.is_whitespace() || c == '\u{feff}' => at += n,
                    _ => return Ok(at),
                },
                _ => return Ok(at),
            }
        }
    }

    /// The directive at `at` (its `#`): an extra, or `None` for a conditional's (`#if` …, a token).
    fn directive(&self, at: usize) -> R<Option<Extra>> {
        if !self.line_start(at) {
            return Err(Refusal::Lex(at as u32));
        }
        let ws = self.blocks.blank_end(at + 1);
        let we = self.blocks.word_end(ws);
        let kind = match &self.src[ws..we] {
            b"if" | b"elif" | b"else" | b"endif" => return Ok(None),
            b"region" => X::Region,
            b"endregion" => X::Endregion,
            b"pragma" => X::Pragma,
            b"nullable" => X::Nullable,
            b"define" => X::Define,
            b"undef" => X::Undef,
            b"error" => X::Error,
            b"warning" => X::Warning,
            b"line" => X::Line,
            _ => return Err(Refusal::Lex(at as u32)),
        };
        // To its `\n`, which it holds.
        let mut end = self.blocks.eol(we);
        while end < self.src.len() && self.src[end] != b'\n' {
            end = self.blocks.eol(end + 1);
        }
        if end >= self.src.len() {
            return Err(Refusal::Lex(at as u32));
        }
        // and the `\n`s right after it (tree-sitter's token there is `\n+`).
        let mut end = end + 1;
        while self.at(end) == b'\n' {
            end += 1;
        }
        Ok(Some(Extra { kind, start: at as u32, end: end as u32, word_end: we as u32 }))
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
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                let end = self.name_end(s)?;
                return Ok((Name, end, word(self.src, s, end)));
            }
            b'0'..=b'9' => return self.number(s),
            b'.' if next.is_ascii_digit() => return self.number(s),
            b'"' => {
                let raw = next == b'"' && self.at(s + 2) == b'"';
                let end = if raw { self.raw_end(s) } else { self.string_end(s) };
                return match end {
                    Some(end) => Ok((if raw { Raw } else { Str }, end, 0)),
                    None => refuse,
                };
            }
            b'@' if next == b'"' => {
                return match self.verbatim_end(s + 1) {
                    Some(end) => Ok((Verbatim, self.u8_suffix(end), 0)),
                    None => refuse,
                };
            }
            b'@' if next == b'$' || (next.is_ascii() && block::is_word(next) && !next.is_ascii_digit()) => {
                if next == b'$' {
                    return match self.interpolated_end(s) {
                        Some(end) => Ok((Interp, end, 0)),
                        None => refuse,
                    };
                }
                // `@name`: a verbatim identifier, never a keyword.
                let end = self.name_end(s + 1)?;
                return Ok((Name, end, 0));
            }
            b'$' => {
                return match self.interpolated_end(s) {
                    Some(end) => Ok((Interp, end, 0)),
                    None => refuse,
                };
            }
            b'\'' => match self.char_end(s) {
                Some(end) => return Ok((Char, end, 0)),
                None => return refuse,
            },
            b'#' => {
                let ws = self.blocks.blank_end(s + 1);
                let we = self.blocks.word_end(ws);
                let kind = match &self.src[ws..we] {
                    b"if" => PpIf,
                    b"elif" => PpElif,
                    b"else" => PpElse,
                    b"endif" => PpEndif,
                    _ => return refuse,
                };
                return Ok((kind, we, 0));
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
            b'?' => match (next, self.at(s + 2)) {
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
            b'.' => {
                if next == b'.' {
                    (DotDot, 2)
                } else {
                    (Dot, 1)
                }
            }
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

    /// The end of a name from `s` (`(\p{XID_Start}|_)\p{XID_Continue}*`): ASCII word bytes a block
    /// a step, a character past ASCII by its class. `s` itself where no name starts there. A
    /// `\u` escape in a name is refused.
    pub fn name_end(&self, s: usize) -> R<usize> {
        let c = self.at(s);
        if c.is_ascii_digit() || (c < 0x80 && !block::is_word(c)) {
            return Ok(s);
        }
        let mut i = s;
        loop {
            i = self.blocks.word_end(i);
            if i >= self.src.len() {
                return Ok(i);
            }
            if self.src[i] == b'\\' && matches!(self.at(i + 1), b'u' | b'U') {
                return Err(Refusal::Lex(i as u32));
            }
            if self.src[i] < 0x80 {
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

    /// `[0-9][0-9_]*[0-9]|[0-9]` from `i` (digits with `_`s between them, by `digit`): its end,
    /// `None` where no digit starts it. Trailing `_`s are not the run's.
    fn digits(&self, i: usize, digit: impl Fn(u8) -> bool) -> Option<usize> {
        if !digit(self.at(i)) {
            return None;
        }
        let mut k = i;
        let mut last = i;
        while digit(self.at(k)) || self.at(k) == b'_' {
            if self.at(k) != b'_' {
                last = k;
            }
            k += 1;
        }
        Some(last + 1)
    }

    /// An exponent from `i` (`[eE][+-]?[0-9][0-9_]*`): its end.
    fn exponent(&self, i: usize) -> Option<usize> {
        if !matches!(self.at(i), b'e' | b'E') {
            return None;
        }
        let mut k = if matches!(self.at(i + 1), b'+' | b'-') { i + 2 } else { i + 1 };
        if !self.at(k).is_ascii_digit() {
            return None;
        }
        while self.at(k).is_ascii_digit() || self.at(k) == b'_' {
            k += 1;
        }
        Some(k)
    }

    /// A number at `s` (a digit, or a `.` before one): the longest of the grammar's literals.
    fn number(&self, s: usize) -> R<(T, usize, u8)> {
        let real_suffix = |i: usize| if matches!(self.at(i), b'f' | b'F' | b'd' | b'D' | b'm' | b'M') { i + 1 } else { i };
        let int_suffix = |i: usize| match (self.at(i), self.at(i + 1)) {
            (b'u' | b'U', b'l' | b'L') | (b'l' | b'L', b'u' | b'U') => i + 2,
            (b'u' | b'U' | b'l' | b'L', _) => i + 1,
            _ => i,
        };
        let dec = |c: u8| c.is_ascii_digit();
        let (c0, c1) = (self.at(s), self.at(s + 1));
        // Hex and binary: `0x[0-9a-fA-F_]*[0-9a-fA-F]+`, `0b[01_]*[01]+`.
        if c0 == b'0' && matches!(c1, b'x' | b'X' | b'b' | b'B') {
            let hex = matches!(c1, b'x' | b'X');
            let digit = |c: u8| if hex { c.is_ascii_hexdigit() } else { c == b'0' || c == b'1' };
            let mut k = s + 2;
            let mut last = None;
            while digit(self.at(k)) || self.at(k) == b'_' {
                if self.at(k) != b'_' {
                    last = Some(k);
                }
                k += 1;
            }
            if let Some(last) = last {
                return Ok((T::Int, int_suffix(last + 1), 0));
            }
        }
        let mut best = (T::Int, s);
        let mut take = |kind: T, end: usize| {
            if end > best.1 {
                best = (kind, end);
            }
        };
        if let Some(e) = self.digits(s, dec) {
            take(T::Int, int_suffix(e));
            if let Some(x) = self.exponent(e) {
                take(T::Real, real_suffix(x));
            }
            if real_suffix(e) > e {
                take(T::Real, e + 1);
            }
        }
        // `D '.' D exponent? suffix?`, and `'.' D …`.
        let int = if c0 == b'.' { Some(s) } else { self.digits(s, dec) };
        if let Some(e) = int {
            if self.at(e) == b'.' {
                if let Some(f) = self.digits(e + 1, dec) {
                    let x = self.exponent(f).unwrap_or(f);
                    take(T::Real, real_suffix(x));
                }
            }
        }
        if best.1 == s {
            return Err(Refusal::Lex(s as u32));
        }
        Ok((best.0, best.1, 0))
    }

    /// The end of an escape sequence from `i` (its `\`): `\x` and 1 to 4 hex digits, `\u` and 4,
    /// `\U` and 8, or `\` and one of `abefnrtv'"\?0`. `None` where none is.
    pub fn escape_end(&self, i: usize) -> Option<usize> {
        let at = |k: usize| self.at(k);
        match at(i + 1) {
            b'x' => {
                let mut k = i + 2;
                while k < i + 6 && at(k).is_ascii_hexdigit() {
                    k += 1;
                }
                (k > i + 2).then_some(k)
            }
            b'u' => (i + 2..i + 6).all(|k| at(k).is_ascii_hexdigit()).then_some(i + 6),
            b'U' => (i + 2..i + 10).all(|k| at(k).is_ascii_hexdigit()).then_some(i + 10),
            b'a' | b'b' | b'e' | b'f' | b'n' | b'r' | b't' | b'v' | b'\'' | b'"' | b'\\' | b'?' | b'0' => Some(i + 2),
            _ => None,
        }
    }

    /// Past a `u8` (`U8`) suffix at `i`, where one is.
    fn u8_suffix(&self, i: usize) -> usize {
        if matches!(self.at(i), b'u' | b'U') && self.at(i + 1) == b'8' { i + 2 } else { i }
    }

    /// A regular string from its `"` at `s`: past its closer (and its `u8`). `None` where a line
    /// or the file ends first, or an escape is not one.
    pub fn string_end(&self, s: usize) -> Option<usize> {
        let mut i = s + 1;
        loop {
            i = self.blocks.string_stop(i);
            let eol = self.blocks.eol(s + 1);
            if eol < i {
                return None;
            }
            match self.src.get(i)? {
                b'"' => return Some(self.u8_suffix(i + 1)),
                _ => i = self.escape_end(i)?,
            }
        }
    }

    /// A verbatim string's body from its `"` at `s` (after `@`): past its closer (`""` passed).
    fn verbatim_end(&self, s: usize) -> Option<usize> {
        let mut i = s + 1;
        loop {
            match self.src.get(i)? {
                b'"' if self.at(i + 1) == b'"' => i += 2,
                b'"' => return Some(i + 1),
                _ => i += 1,
            }
        }
    }

    /// The quotes opening a raw string at `s`: their count.
    pub fn quotes(&self, s: usize) -> usize {
        let mut n = 0;
        while self.at(s + n) == b'"' {
            n += 1;
        }
        n
    }

    /// A raw string from its quotes at `s` (3 or more): past its closer, a run of as many quotes
    /// (and its `u8`).
    fn raw_end(&self, s: usize) -> Option<usize> {
        let n = self.quotes(s);
        let e = self.raw_content_end(s + n, n)?;
        Some(self.u8_suffix(e + n))
    }

    /// Where a raw string's content ends from `i`: at a run of `n` quotes or more.
    pub fn raw_content_end(&self, mut i: usize, n: usize) -> Option<usize> {
        loop {
            if i >= self.src.len() {
                return None;
            }
            if self.src[i] == b'"' {
                let q = self.quotes(i);
                if q >= n {
                    return (q == n).then_some(i);
                }
                i += q;
            } else {
                i += 1;
            }
        }
    }

    /// An interpolated string's start at `s`: its `$`s, whether verbatim, its opening quotes'
    /// count, and where its content starts. `None` where none starts there.
    pub fn interpolation_start(&self, s: usize) -> Option<(usize, bool, usize, usize)> {
        let mut i = s;
        let mut verbatim = false;
        if self.at(i) == b'@' {
            verbatim = true;
            i += 1;
        }
        let mut dollars = 0;
        while self.at(i) == b'$' {
            dollars += 1;
            i += 1;
        }
        if !verbatim && self.at(i) == b'@' {
            verbatim = true;
            i += 1;
        }
        if dollars == 0 || self.at(i) != b'"' {
            return None;
        }
        let q = self.quotes(i);
        let quotes = if q >= 3 && !verbatim { q } else { 1 };
        if (dollars > 1 && quotes == 1) || (verbatim && q >= 3 && q != 3) {
            return None;
        }
        Some((dollars, verbatim, quotes, i + quotes))
    }

    /// An interpolated string from `s`: past its closer.
    fn interpolated_end(&self, s: usize) -> Option<usize> {
        let (dollars, verbatim, quotes, mut i) = self.interpolation_start(s)?;
        loop {
            match *self.src.get(i)? {
                b'"' if quotes > 1 => {
                    let q = self.quotes(i);
                    if q >= quotes {
                        return (q == quotes).then_some(i + q);
                    }
                    i += q;
                }
                b'"' if verbatim && self.at(i + 1) == b'"' => i += 2,
                b'"' => return Some(i + 1),
                b'\\' if !verbatim && quotes == 1 => i = self.escape_end(i)?,
                b'\n' if !verbatim && quotes == 1 => return None,
                b'{' => {
                    let mut b = 0;
                    while self.at(i + b) == b'{' {
                        b += 1;
                    }
                    if quotes == 1 && dollars == 1 && b >= 2 {
                        // `{{`: text.
                        i += 2;
                        continue;
                    }
                    if b < dollars {
                        i += b;
                        continue;
                    }
                    // The last `dollars` braces open the interpolation.
                    i = self.hole_end(i + b)?;
                }
                b'}' if quotes == 1 && dollars == 1 && self.at(i + 1) == b'}' => i += 2,
                _ => i += 1,
            }
        }
    }

    /// From inside an interpolation (past its braces): past its closing `}`s, nested brackets,
    /// strings and characters passed.
    fn hole_end(&self, mut i: usize) -> Option<usize> {
        let mut depth = 0u32;
        loop {
            match *self.src.get(i)? {
                b'{' => depth += 1,
                b'}' if depth == 0 => {
                    let mut b = 0;
                    while self.at(i + b) == b'}' {
                        b += 1;
                    }
                    return Some(i + b);
                }
                b'}' => depth -= 1,
                b'"' => {
                    i = if self.at(i + 1) == b'"' && self.at(i + 2) == b'"' { self.raw_end(i)? } else { self.string_end(i)? };
                    continue;
                }
                b'@' if self.at(i + 1) == b'"' => {
                    i = self.verbatim_end(i + 1)?;
                    continue;
                }
                b'@' | b'$' if self.interpolation_start(i).is_some() => {
                    i = self.interpolated_end(i)?;
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

    /// A character literal from `s` (its `'`): one character but `'` and `\`, or an escape
    /// sequence, then `'`. Its end.
    fn char_end(&self, s: usize) -> Option<usize> {
        let i = s + 1;
        let e = match self.at(i) {
            b'\\' => self.escape_end(i)?,
            b'\'' | b'\n' | b'\r' => return None,
            c if c >= 0x80 => i + decode(self.src, i)?.1,
            _ if i >= self.src.len() => return None,
            _ => i + 1,
        };
        (self.at(e) == b'\'').then_some(e + 1)
    }
}
