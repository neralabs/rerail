//! The lexer: tree-sitter-swift's lexer and its external scanner, read where the parser asks. The
//! lexer keeps no state: the parser asks for the next token at a byte (`Lexer::token`).
//!
//! tree-sitter-swift has no keyword extraction: a word is a keyword where the parse state takes
//! the keyword, else a name. Every word is a `Name` with its word code (`w::…`; 0: none) from one
//! probe of the keyword switch, and the parser decides.
//!
//! An operator is the run of operator characters the scanner reads (`is_legal_custom_operator`):
//! a run that is one of the grammar's operators is that operator, any other a custom operator, as
//! the scanner's reserved operators decide. The parser splits a run where its state reads a shorter
//! token (`>` closing type arguments in `>>`, `?` after a type in `>?`).
//!
//! The scanner's implicit semicolon is read from the bytes after a token (`Lexer::semi`), where
//! the parser's state takes one, with the scanner's valid keywords (`SemiCtx`).

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

/// A token's kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum T {
    Eof,
    /// A name: `[_\p{XID_Start}][_\p{XID_Continue}]*`, a backtick name, `$0` or `$name` (word code
    /// `w::NONE` and `Tok::tick` where it is never a keyword).
    Name,
    Int,
    Hex,
    Oct,
    Bin,
    Real,
    /// A string literal, `"`, `"""` or a raw string's `#`s, to its end (its pieces are read by the
    /// parser).
    Str,
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Semi,
    Comma,
    Colon,
    At,
    /// `#` before a word (a directive's, a macro's, a special literal's).
    Hash,
    /// `\` (a key path's).
    Backslash,
    /// `#!` at the file's start.
    Shebang,
    // Operators: a run of operator characters.
    Dot,
    DotDot,
    DotDotDot,
    DotDotLt,
    Arrow,
    Eq,
    EqEq,
    EqEqEq,
    Bang,
    Question,
    QQ,
    AmpAmp,
    PipePipe,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Lt,
    Gt,
    Amp,
    Pipe,
    Caret,
    Tilde,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    PlusPlus,
    MinusMinus,
    LtLt,
    GtGt,
    /// Any other run: `custom_operator` (`!=`, `<=`, `>=`, `!==`, `~=`, …).
    Custom,
}

#[derive(Clone, Copy, Debug)]
pub struct Tok {
    pub kind: T,
    /// A name's word code (`w::…`; 0: none).
    pub word: u8,
    /// A line end in the white space before the token (outside comments).
    pub nl: bool,
    /// A name that is never a keyword (a backtick name, `$0`, a name past ASCII).
    pub tick: bool,
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
        PROTOCOL_T = b"Protocol", TYPE_T = b"Type", UNDERSCORE = b"_", MODIFY = b"_modify", ACTOR = b"actor",
        ANY = b"any", ARCH = b"arch", ASSOCIATEDTYPE = b"associatedtype", ASYNC = b"async",
        AVAILABLE = b"available", AWAIT = b"await", BORROWING = b"borrowing", BREAK = b"break",
        CANIMPORT = b"canImport", CASE = b"case", CLASS = b"class", COLORLITERAL = b"colorLiteral",
        COLUMN = b"column", COMPILER = b"compiler", CONSUME = b"consume", CONSUMING = b"consuming",
        CONTINUE = b"continue", CONVENIENCE = b"convenience", COPY = b"copy", DEINIT = b"deinit",
        DELEGATE = b"delegate", DIDSET = b"didSet", DISCARD = b"discard", DISTRIBUTED = b"distributed",
        DO = b"do", DSOHANDLE = b"dsohandle", DYNAMIC = b"dynamic", EACH = b"each", ENUM = b"enum",
        EXTENSION = b"extension", EXTERNALMACRO = b"externalMacro", FALLTHROUGH = b"fallthrough",
        FALSE = b"false", FILE = b"file", FILEID = b"fileID", FILELITERAL = b"fileLiteral",
        FILEPATH = b"filePath", FILEPRIVATE = b"fileprivate", FINAL = b"final", FOR = b"for",
        FUNC = b"func", FUNCTION = b"function", GET = b"get", GUARD = b"guard", IF = b"if",
        IMAGELITERAL = b"imageLiteral", IMPORT = b"import", IN = b"in", INDIRECT = b"indirect",
        INFIX = b"infix", INIT = b"init", INOUT = b"inout", INTERNAL = b"internal", IS = b"is",
        ISOLATED = b"isolated", KEYPATH = b"keyPath", LAZY = b"lazy", LET = b"let", LINE = b"line",
        MACRO = b"macro", MUTATING = b"mutating", NIL = b"nil", NONISOLATED = b"nonisolated",
        NONMUTATING = b"nonmutating", NONSENDING = b"nonsending", OF = b"of", OPEN = b"open",
        OPERATOR = b"operator", OPTIONAL = b"optional", OS = b"os", OVERRIDE = b"override",
        PACKAGE = b"package", PARAM = b"param", POSTFIX = b"postfix",
        PRECEDENCEGROUP = b"precedencegroup", PREFIX = b"prefix", PRIVATE = b"private",
        PROPERTY = b"property", PROTOCOL = b"protocol", PUBLIC = b"public", RECEIVER = b"receiver",
        REPEAT = b"repeat", REQUIRED = b"required", RETURN = b"return", SELECTOR = b"selector",
        SELF = b"self", SENDING = b"sending", SET = b"set", SETPARAM = b"setparam", SOME = b"some",
        STATIC = b"static", STRUCT = b"struct", SUBSCRIPT = b"subscript", SUPER = b"super",
        SWIFT = b"swift", SWITCH = b"switch", TARGETENVIRONMENT = b"targetEnvironment",
        THROW = b"throw", TRUE = b"true", TRY = b"try", TYPEALIAS = b"typealias", U = b"u",
        UNAVAILABLE = b"unavailable", UNOWNED = b"unowned", UNSAFE = b"unsafe", UNUSED1 = b"unused1",
        UNUSED2 = b"unused2", VAR = b"var", WEAK = b"weak", WHILE = b"while", WILLSET = b"willSet",
        YIELD = b"yield",
        // The scanner's keywords.
        THROWS = b"throws", RETHROWS = b"rethrows", DEFAULT = b"default", WHERE = b"where",
        ELSE = b"else", CATCH = b"catch", AS = b"as",
        // Directives' words (after `#`).
        ELSEIF = b"elseif", ENDIF = b"endif", ERROR = b"error", WARNING = b"warning",
        SOURCELOCATION = b"sourceLocation",
    }
}

rerail_simd8::keyword_switch!(
    /// The keyword switch (`rerail_simd8::words`) over `w::TEXT`: a word's code is its index there.
    static WORDS = w::TEXT, fold: false
);

/// The word `src[i..end]`'s code (0: none): one probe of the keyword switch.
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

/// Whether `c` (past ASCII) is a name's character the lexer reads for certain (`XID_Continue` in
/// the scripts where it is `Alphabetic`); `None` where it is not certain (refused).
fn letter(c: char) -> Option<bool> {
    let u = c as u32;
    let sure = matches!(u, 0xc0..=0x24f | 0x370..=0x3ff | 0x400..=0x4ff | 0x3040..=0x30ff | 0x4e00..=0x9fff | 0xac00..=0xd7a3)
        && u != 0xd7
        && u != 0xf7
        && u != 0x37e
        && u != 0x387;
    if sure {
        Some(c.is_alphabetic())
    } else if c.is_whitespace() || matches!(u, 0x2000..=0x206f) {
        Some(false)
    } else {
        None
    }
}

#[inline(always)]
fn space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

#[inline(always)]
fn wordc(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// An operator character (`is_legal_custom_operator` at the run's start, ASCII).
#[inline(always)]
pub fn op_char(c: u8) -> bool {
    matches!(c, b'/' | b'=' | b'-' | b'+' | b'!' | b'*' | b'%' | b'<' | b'>' | b'&' | b'|' | b'^' | b'?' | b'~' | b'.')
}

/// The operator symbols a fixed operator may not be followed by (`OPERATOR_SYMBOLS`).
#[inline(always)]
fn op_symbol(c: u8) -> bool {
    matches!(c, b'/' | b'=' | b'-' | b'+' | b'!' | b'*' | b'%' | b'<' | b'>' | b'&' | b'|' | b'^' | b'?' | b'~')
}

/// The scanner's keywords and operators valid where the parser asks for a semicolon: a line end
/// before one of them continues what is read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SemiCtx {
    /// `else` (after an `if`'s block, a `guard`'s conditions).
    pub else_: bool,
    /// `catch` (after a `do`'s block or catch).
    pub catch: bool,
    /// `default` (in a switch's entry).
    pub default: bool,
    /// `where`.
    pub where_: bool,
    /// `->`, `throws`, `rethrows`, `async` (after a function's parameters).
    pub arrow: bool,
    /// After an expression: `.`, `=`, `==`, `+ `, `- `, `as`, a custom operator continue it.
    pub expr: bool,
}

/// The implicit semicolon where the parser's state takes one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Semi {
    /// None: the token after continues.
    No,
    /// A semicolon of no width at this byte (after a line end's white space).
    Implicit(u32),
    /// A `;` at this byte.
    Explicit(u32),
    /// A block comment at this byte on a line after the token: the scanner reads it as a token (an
    /// extra), and is asked again after it.
    Comment(u32),
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
            let Some((end, block)) = self.comment(at)? else { return Ok((at, nl)) };
            let kind = if block { crate::K::MultilineComment } else { crate::K::Comment };
            extras.push((crate::tw::key(kind, 0, true), at as u32, end as u32));
            at = end;
        }
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
    fn line_end(&self, i: usize) -> usize {
        let mut end = self.blocks.eol(i);
        while end < self.src.len() && self.src[end] != b'\n' {
            end = self.blocks.eol(end + 1);
        }
        end
    }

    /// The end of the (nested) block comment at `at` (its `/*`); `None` where the file ends first.
    pub fn block_comment_end(&self, at: usize) -> Option<usize> {
        let mut i = at + 2;
        let mut depth = 1u32;
        let mut after_star = false;
        loop {
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
                let (end, wide) = self.name_end(s)?;
                let code = if wide { 0 } else { word(self.src, s, end) };
                return Ok((Name, end, code, wide));
            }
            b'`' => {
                let mut i = s + 1;
                while i < self.src.len() && !matches!(self.src[i], b'`' | b'\n' | b'\r') {
                    i += 1;
                }
                if i == s + 1 || self.at(i) != b'`' {
                    return refuse;
                }
                return Ok((Name, i + 1, 0, true));
            }
            b'$' => {
                // `$0`, or `$name`.
                if next.is_ascii_digit() {
                    let mut i = s + 1;
                    while self.at(i).is_ascii_digit() {
                        i += 1;
                    }
                    return Ok((Name, i, 0, true));
                }
                let (end, _) = self.name_end(s + 1)?;
                if end == s + 1 {
                    return refuse;
                }
                return Ok((Name, end, 0, true));
            }
            b'0'..=b'9' => return self.number(s),
            b'"' => {
                return match self.string_end(s, 0) {
                    Some(end) => Ok((Str, end, 0, false)),
                    None => refuse,
                };
            }
            b'#' => {
                if s == 0 && next == b'!' {
                    (Shebang, 2)
                } else {
                    let mut k = s;
                    while self.at(k) == b'#' {
                        k += 1;
                    }
                    if self.at(k) == b'"' {
                        return match self.string_end(k, (k - s) as u32) {
                            Some(end) => Ok((Str, end, 0, false)),
                            None => refuse,
                        };
                    }
                    if k > s + 1 {
                        return refuse;
                    }
                    (Hash, 1)
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
            b':' => (Colon, 1),
            b'@' => (At, 1),
            b'\\' => (Backslash, 1),
            c if op_char(c) => {
                let (kind, end) = self.operator(s)?;
                return Ok((kind, end, 0, false));
            }
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

    /// The operator run at `s` (`is_legal_custom_operator`): a `.` only where the run starts with
    /// one; no `//` or `/*`. Its kind: the grammar's operator of its text, else a custom operator.
    pub fn operator(&self, s: usize) -> R<(T, usize)> {
        use T::*;
        let first = self.src[s];
        let mut i = s + 1;
        loop {
            let c = self.at(i);
            let ok = match c {
                b'.' => first == b'.',
                b'*' | b'/' => !(i == s + 1 && first == b'/'),
                c => op_char(c),
            };
            if !ok {
                break;
            }
            i += 1;
        }
        if self.at(i) >= 0x80 {
            // An operator character past ASCII (or a combining mark) may go on: refused.
            if decode(self.src, i).is_some_and(|(c, _)| !c.is_alphanumeric() && !c.is_whitespace()) {
                return Err(Refusal::Lex(i as u32));
            }
        }
        let kind = match &self.src[s..i] {
            b"." => Dot,
            b".." => DotDot,
            b"..." => DotDotDot,
            b"..<" => DotDotLt,
            b"->" => Arrow,
            b"=" => Eq,
            b"==" => EqEq,
            b"===" => EqEqEq,
            b"!" => Bang,
            b"?" => Question,
            b"??" => QQ,
            b"&&" => AmpAmp,
            b"||" => PipePipe,
            b"+" => Plus,
            b"-" => Minus,
            b"*" => Star,
            b"/" => Slash,
            b"%" => Percent,
            b"<" => Lt,
            b">" => Gt,
            b"&" => Amp,
            b"|" => Pipe,
            b"^" => Caret,
            b"~" => Tilde,
            b"+=" => PlusEq,
            b"-=" => MinusEq,
            b"*=" => StarEq,
            b"/=" => SlashEq,
            b"%=" => PercentEq,
            b"++" => PlusPlus,
            b"--" => MinusMinus,
            b"<<" => LtLt,
            b">>" => GtGt,
            b"*/" => return Err(Refusal::Lex(s as u32)),
            t => {
                // A custom operator ending in `<` before no white space ends before it (the
                // scanner's mark): refused.
                if t.last() == Some(&b'<') && !space(self.at(i)) {
                    return Err(Refusal::Lex(s as u32));
                }
                Custom
            }
        };
        Ok((kind, i))
    }

    /// The end of a name from `s`: ASCII word bytes a block a step, a character past ASCII by its
    /// class (refused where the class is not certain). `s` itself where no name starts there; and
    /// whether a character past ASCII is in it.
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

    /// Digits from `i` (runs of `digit` with runs of `_` between them): the end, `None` where no
    /// digit starts it.
    fn digits(&self, mut i: usize, digit: impl Fn(u8) -> bool) -> Option<usize> {
        if !digit(self.at(i)) {
            return None;
        }
        loop {
            while digit(self.at(i)) {
                i += 1;
            }
            let mut k = i;
            while self.at(k) == b'_' {
                k += 1;
            }
            if k > i && digit(self.at(k)) {
                i = k;
                continue;
            }
            return Some(i);
        }
    }

    fn exponent(&self, i: usize, e: [u8; 2]) -> Option<usize> {
        if !e.contains(&self.at(i)) {
            return None;
        }
        let k = if matches!(self.at(i + 1), b'+' | b'-') { i + 2 } else { i + 1 };
        self.digits(k, |c| c.is_ascii_digit())
    }

    /// A number at `s` (a digit): the longest of the grammar's literals.
    fn number(&self, s: usize) -> R<(T, usize, u8, bool)> {
        let dec = |c: u8| c.is_ascii_digit();
        let hex = |c: u8| c.is_ascii_hexdigit();
        let (c0, c1) = (self.at(s), self.at(s + 1));
        let mut best = (T::Int, s);
        if c0 == b'0' && matches!(c1, b'x' | b'X') {
            if let Some(e) = self.digits(s + 2, hex) {
                best = (T::Hex, e);
                // A hex float: `0x` digits (`.` digits)? `p` exponent (only `0x`, not `0X`).
                if c1 == b'x' {
                    let f = if self.at(e) == b'.' { self.digits(e + 1, hex) } else { Some(e) };
                    if let Some(x) = f.and_then(|f| self.exponent(f, [b'p', b'P'])) {
                        best = (T::Real, x);
                    }
                }
            }
        } else if c0 == b'0' && matches!(c1, b'o' | b'O') {
            if let Some(e) = self.digits(s + 2, |c| (b'0'..=b'7').contains(&c)) {
                best = (T::Oct, e);
            }
        } else if c0 == b'0' && matches!(c1, b'b' | b'B') {
            if let Some(e) = self.digits(s + 2, |c| c == b'0' || c == b'1') {
                best = (T::Bin, e);
            }
        }
        if best.1 == s {
            let e = self.digits(s, dec).expect("a digit");
            best = (T::Int, e);
            if let Some(x) = self.exponent(e, [b'e', b'E']) {
                best = (T::Real, x);
            }
            if self.at(e) == b'.' {
                if let Some(f) = self.digits(e + 1, dec) {
                    let x = self.exponent(f, [b'e', b'E']).unwrap_or(f);
                    best = (T::Real, x);
                }
            }
        }
        // A word byte right after the number: the lexer reads another token there (refused).
        if wordc(self.at(best.1)) {
            return Err(Refusal::Lex(s as u32));
        }
        Ok((best.0, best.1, 0, false))
    }

    /// The end of a string literal from its `"` at `s`, after `hashes` `#`s (a raw string): past
    /// its closer. An interpolation's (`\(…)`, `\#(…)`) brackets and strings are passed. `None`
    /// where the line or the file ends first (a line string), or the file (a multi-line one).
    pub fn string_end(&self, s: usize, hashes: u32) -> Option<usize> {
        let triple = self.src[s..].starts_with(b"\"\"\"");
        let mut i = s + if triple { 3 } else { 1 };
        let h = hashes as usize;
        let n = self.src.len();
        // A one-line string ends on its line: its stops are before the line's end.
        let mut eol = if triple || h > 0 { n } else { self.blocks.eol(i) };
        loop {
            // The next `"`, `\` (or `$`, Kotlin's, passed): the bytes between are text.
            let j = self.blocks.string_stop(i);
            if j >= n || j > eol {
                return None;
            }
            i = j;
            match self.src[j] {
                b'"' => {
                    let closes = if triple { self.src[j..].starts_with(b"\"\"\"") } else { true };
                    let q = if triple { 3 } else { 1 };
                    if closes && self.src.len() >= j + q + h && self.src[j + q..j + q + h].iter().all(|&c| c == b'#') {
                        return Some(j + q + h);
                    }
                    i += 1;
                }
                b'\\' => {
                    if self.src.len() >= j + 1 + h && self.src[j + 1..j + 1 + h].iter().all(|&c| c == b'#') {
                        let k = j + 1 + h;
                        if self.at(k) == b'(' {
                            i = self.interpolation_end(k + 1)?;
                            if eol < i && eol != n {
                                // An interpolation may hold a line end: the line is the one it ends on.
                                eol = self.blocks.eol(i);
                            }
                            continue;
                        }
                        if h == 0 {
                            if j + 1 >= n {
                                return None;
                            }
                            i += 2;
                            continue;
                        }
                    }
                    i += 1;
                }
                _ => i += 1,
            }
        }
    }

    /// From inside an interpolation (past its `(`): past its `)`, nested brackets, strings and
    /// comments passed.
    fn interpolation_end(&self, mut i: usize) -> Option<usize> {
        let mut depth = 0u32;
        loop {
            match self.src.get(i)? {
                b'(' => depth += 1,
                b')' if depth == 0 => return Some(i + 1),
                b')' => depth -= 1,
                b'"' => {
                    i = self.string_end(i, 0)?;
                    continue;
                }
                b'#' if matches!(self.at(i + 1), b'#' | b'"') => {
                    let mut k = i;
                    while self.at(k) == b'#' {
                        k += 1;
                    }
                    if self.at(k) == b'"' {
                        i = self.string_end(k, (k - i) as u32)?;
                        continue;
                    }
                }
                b'/' if matches!(self.at(i + 1), b'/' | b'*') => {
                    let (end, _) = self.comment(i).ok()??;
                    i = end;
                    continue;
                }
                _ => {}
            }
            i += 1;
        }
    }

    // ---- The external scanner's semicolon.

    /// Whether the word `w` is at `i` and ends there.
    pub fn word_at(&self, i: usize, w: &[u8]) -> bool {
        self.src[i.min(self.src.len())..].starts_with(w) && !wordc(self.at(i + w.len()))
    }

    /// Whether the scanner reads an operator at `i` that suppresses the line end
    /// (`eat_operators` with `is_cross_semi_token`), in context `cx`; `any`: any operator it reads
    /// (`eat_whitespace`'s look past comments).
    fn cross_semi(&self, i: usize, cx: SemiCtx, any: bool) -> bool {
        let c = self.at(i);
        let next = self.at(i + 1);
        if c == b'-' && cx.arrow && next == b'>' && !op_symbol(self.at(i + 2)) {
            return true;
        }
        if cx.expr {
            if c == b'.' {
                return next != b'.' && !op_symbol(next);
            }
            if matches!(c, b'+' | b'-') && space(next) {
                return true;
            }
            if self.word_at(i, b"as") || self.src[i..].starts_with(b"as?") || self.src[i..].starts_with(b"as!") {
                return true;
            }
            if op_char(c) {
                return match self.operator(i) {
                    Ok((T::Custom | T::Eq | T::EqEq, _)) => true,
                    // Read by the scanner, not across a line end: the line end is looked past
                    // only from a comment.
                    Ok((T::AmpAmp | T::PipePipe | T::QQ | T::Bang, _)) => any,
                    _ => false,
                };
            }
        }
        (cx.else_ && self.word_at(i, b"else"))
            || (cx.catch && self.word_at(i, b"catch"))
            || (cx.default && self.word_at(i, b"default"))
            || (cx.where_ && self.word_at(i, b"where"))
            || (cx.arrow && (self.word_at(i, b"throws") || self.word_at(i, b"rethrows") || self.word_at(i, b"async")))
    }

    /// The semicolon after a token ending at `from`, where the parser's state takes one
    /// (`eat_whitespace`, then `eat_operators`), with the scanner's valid symbols `cx`.
    pub fn semi(&self, from: usize, cx: SemiCtx) -> Semi {
        let n = self.src.len();
        let mut i = from;
        let mut nl = false;
        // The end of the whitespace before the first comment after a newline (the file's end's
        // semicolon is there).
        let mut mark = None;
        // A comment after the newline: the operators that do not cross a line alone cross it.
        let mut comment = false;
        loop {
            while i < n && space(self.src[i]) {
                nl |= matches!(self.src[i], b'\n' | b'\r');
                i += 1;
            }
            if self.at(i) != b'/' {
                break;
            }
            match self.at(i + 1) {
                b'/' => {
                    if nl {
                        comment = true;
                        mark.get_or_insert(i);
                    }
                    while i < n && self.src[i] != b'\n' {
                        i += 1;
                    }
                }
                b'*' if nl => return Semi::Comment(i as u32),
                b'*' => {
                    let Some(e) = self.block_comment_end(i) else { return Semi::No };
                    i = e;
                }
                // A `/` operator: the scanner declines (a division goes on).
                _ => return Semi::No,
            }
        }
        if self.at(i) == b';' {
            return Semi::Explicit(i as u32);
        }
        if i >= n {
            // The file's end: tree-sitter's end of input.
            return if nl { Semi::Implicit(mark.unwrap_or(i) as u32) } else { Semi::No };
        }
        if !nl {
            return Semi::No;
        }
        if !comment && matches!(self.at(i), b'?' | b':' | b'{' | b'&' | b'|' | b'^' | b'<' | b'>') {
            return Semi::No;
        }
        if self.cross_semi(i, cx, comment) {
            return Semi::No;
        }
        Semi::Implicit(i as u32)
    }

}
