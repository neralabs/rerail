//! The preprocessor's reading of a file (decision 0327): translation phases 1 and 2, then
//! preprocessing tokens (C17 §5.1.1.2 and §6.4, C++ [lex.phases] and [lex.pptoken]).
//!
//! The parser's lexer (`lex`) reads tree-sitter's tokens, and refuses what tree-sitter reads
//! differently. The preprocessor needs the standard's tokens, and never refuses: an apostrophe in a
//! skipped group, a stray `@` or a byte that starts no token is a token of its own (`P::Other`).
//! Both read with the same pieces: stage 1's masks (`Blocks`), the gap and comment scans, the
//! micro-lexers, names by their word run, raw strings by `raw_end`.
//!
//! **Phases 1 and 2** (`Phase2`). A `\`-newline is removed wherever it is, inside a token too, and
//! trigraphs are replaced where the dialect reads them. A file with neither is read in place; one
//! with any is copied once with them removed, and `Phase2::orig` maps a position of the copy back
//! to the file's.

use crate::lex::{Extra, Lexer, NO_NL, T};

/// A preprocessing token's kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum P {
    /// The text's end.
    Eof,
    Ident,
    /// A pp-number: a digit or `.` and a digit, then digits, identifier characters, `.`, `'` before
    /// a digit or non-digit (where the dialect has digit separators), and `e+` `e-` `p+` `p-`.
    Number,
    /// A character literal, its prefix and (C++) its suffix included.
    Char,
    /// A string literal, its prefix and (C++) its suffix included.
    Str,
    /// A raw string literal, its prefix and suffix included.
    RawStr,
    /// `#` (or `%:`).
    Hash,
    /// `##` (or `%:%:`).
    HashHash,
    /// Any other punctuator: its kind is the token's `punct`.
    Punct,
    /// A header name, `<…>` or `"…"`, read only where an include asks (`PpLexer::header_name`).
    Header,
    /// A byte or character that starts no other token, or an unclosed quote.
    Other,
}

/// `PpTok::flags`: the first token of a logical line...
pub const BOL: u8 = 1;
/// ...white space (or a comment) before it on its line...
pub const SPACE: u8 = 2;
/// ...a digraph's spelling (`<:`, `:>`, `<%`, `%>`, `%:`, `%:%:`)...
pub const DIGRAPH: u8 = 4;
/// ...a quote with no close on its line (an `Other`)...
pub const UNCLOSED: u8 = 8;
/// ...and a block comment with no close, which the text's end ends (an `Eof`).
pub const UNCLOSED_COMMENT: u8 = 16;

/// A preprocessing token: its kind, its punctuator's kind (`T::Eof` for any other), its flags and
/// its bytes in the phase-2 text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PpTok {
    pub kind: P,
    pub punct: T,
    pub flags: u8,
    pub start: u32,
    pub end: u32,
}

impl PpTok {
    /// Whether this is punctuator `t`.
    #[inline]
    pub fn is(&self, t: T) -> bool {
        self.kind == P::Punct && self.punct == t
    }
}

/// How a dialect reads its tokens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    /// C++: `::`, `.*`, `->*`, `<=>`, raw strings, user-defined literals' suffixes, `<::`'s rule.
    pub cpp: bool,
    /// `'` between a pp-number's digits (C++14, C23).
    pub digit_separators: bool,
    /// Trigraphs replaced in phase 1 (C before C23, C++ before C++17, or asked for).
    pub trigraphs: bool,
    /// Raw string literals (C++11; a GNU extension in C).
    pub raw_strings: bool,
    /// `::` one punctuator (C++, and C23 for attributes).
    pub colons: bool,
}

impl Options {
    /// C at standard year `std` (89, 99, 11, 17, 23), with GNU's extensions where `gnu`.
    pub fn c(std: u32, gnu: bool) -> Self {
        Options { cpp: false, digit_separators: std >= 23, trigraphs: std < 23 && !gnu, raw_strings: gnu, colons: std >= 23 }
    }

    /// C++ at standard year `std` (98, 11, 14, 17, 20, 23, 26), with GNU's extensions where `gnu`.
    pub fn cpp(std: u32, gnu: bool) -> Self {
        let std = if std < 50 { std + 2000 } else { std + 1900 };
        Options { cpp: true, digit_separators: std >= 2014, trigraphs: std < 2017 && !gnu, raw_strings: std >= 2011, colons: true }
    }
}

/// A file after translation phases 1 and 2: its text, and where bytes were removed.
pub struct Phase2<'s> {
    pub text: std::borrow::Cow<'s, [u8]>,
    /// For each place bytes were removed: its position in `text`, and how many bytes were removed
    /// up to and there (ascending).
    removed: Vec<(u32, u32)>,
}

impl<'s> Phase2<'s> {
    /// `src` with its `\`-newlines removed (a `\`, blanks, then `\r\n` or `\n`: the blanks are
    /// clang's and GCC's extension), its trigraphs replaced where `trigraphs`, and a UTF-8 byte
    /// order mark at its start passed (as clang passes it).
    pub fn of(src: &'s [u8], trigraphs: bool) -> Self {
        let bom = src.starts_with(b"\xef\xbb\xbf");
        let has_trigraph = trigraphs && rerail_simd8::micro::find_pair(src, 0, b'?', b'?').is_some();
        let first = first_splice(src, 0);
        if first.is_none() && !has_trigraph && !bom {
            return Phase2 { text: std::borrow::Cow::Borrowed(src), removed: Vec::new() };
        }
        let mut text = Vec::with_capacity(src.len());
        let mut removed = Vec::new();
        let mut gone = 0u32;
        let mut i = 0;
        if bom {
            gone = 3;
            removed.push((0, 3));
            i = 3;
        }
        while i < src.len() {
            let c = src[i];
            if c == b'?' && trigraphs && src.get(i + 1) == Some(&b'?') {
                if let Some(r) = src.get(i + 2).and_then(|&t| trigraph(t)) {
                    if r == b'\\' {
                        if let Some(e) = splice_at(src, i + 2) {
                            // `??/` and a newline: a splice.
                            gone += (e - i) as u32;
                            removed.push((text.len() as u32, gone));
                            i = e;
                            continue;
                        }
                    }
                    text.push(r);
                    gone += 2;
                    removed.push((text.len() as u32, gone));
                    i += 3;
                    continue;
                }
            }
            if c == b'\\' {
                if let Some(e) = splice_at(src, i) {
                    gone += (e - i) as u32;
                    removed.push((text.len() as u32, gone));
                    i = e;
                    continue;
                }
            }
            text.push(c);
            i += 1;
        }
        Phase2 { text: std::borrow::Cow::Owned(text), removed }
    }

    /// The file's position of `text`'s position `at`.
    pub fn orig(&self, at: usize) -> usize {
        let k = self.removed.partition_point(|&(p, _)| p as usize <= at);
        at + if k == 0 { 0 } else { self.removed[k - 1].1 as usize }
    }

    /// Whether the text is the file's own bytes.
    pub fn in_place(&self) -> bool {
        self.removed.is_empty()
    }
}

/// The trigraph `??t`'s replacement.
fn trigraph(t: u8) -> Option<u8> {
    Some(match t {
        b'=' => b'#',
        b'(' => b'[',
        b'/' => b'\\',
        b')' => b']',
        b'\'' => b'^',
        b'<' => b'{',
        b'!' => b'|',
        b'>' => b'}',
        b'-' => b'~',
        _ => return None,
    })
}

/// The end of a splice whose `\` is at `i`: blanks, then `\r\n` or `\n`.
fn splice_at(src: &[u8], i: usize) -> Option<usize> {
    let mut k = i + 1;
    while matches!(src.get(k), Some(b' ' | b'\t' | 0x0b | 0x0c)) {
        k += 1;
    }
    match src.get(k) {
        Some(b'\n') => Some(k + 1),
        Some(b'\r') if src.get(k + 1) == Some(&b'\n') => Some(k + 2),
        _ => None,
    }
}

/// The first splice from `i`, by the micro-lexer's scan for `\`.
fn first_splice(src: &[u8], mut i: usize) -> Option<usize> {
    loop {
        let k = rerail_simd8::micro::find(src, i, b'\\')?;
        if splice_at(src, k).is_some() {
            return Some(k);
        }
        i = k + 1;
    }
}

/// The preprocessing tokens of a phase-2 text, one at a time.
pub struct PpLexer<'t> {
    lx: Lexer<'t>,
    opts: Options,
    /// Where the next token is read from.
    pub pos: usize,
    /// No token read yet on the current line (a file starts with one).
    bol: bool,
    /// The comments passed (cleared at each token: kept only for the scan's sake).
    extras: Vec<Extra>,
}

impl<'t> PpLexer<'t> {
    /// A lexer over `text` (a phase-2 text) from its start.
    pub fn new(text: &'t [u8], opts: Options) -> Self {
        PpLexer { lx: Lexer::new(text, opts.cpp), opts, pos: 0, bol: true, extras: Vec::new() }
    }

    /// The text.
    #[inline]
    pub fn text(&self) -> &'t [u8] {
        self.lx.src
    }

    #[inline]
    fn at(&self, i: usize) -> u8 {
        self.lx.at(i)
    }

    /// The next token.
    pub fn next(&mut self) -> PpTok {
        let (s, flags) = self.gap();
        let t = self.token_at(s, flags);
        self.pos = t.end as usize;
        self.bol = false;
        t
    }

    /// The gap from `pos` passed: where the next token starts, and its `BOL` and `SPACE` flags.
    fn gap(&mut self) -> (usize, u8) {
        self.extras.clear();
        let from = self.pos;
        let (s, nl, unclosed) = match self.lx.skip(from, &mut self.extras) {
            Ok((s, nl)) => (s, nl, false),
            // An unclosed block comment runs to the text's end.
            Err(_) => (self.lx.src.len(), NO_NL, true),
        };
        let bol = self.bol || nl != NO_NL;
        // White space on the token's own line: after the last line end of the gap, or all of it.
        let line_from = if nl == NO_NL { from } else { self.last_eol(from, s).map_or(from, |e| e + 1) };
        let mut flags = if bol { BOL } else { 0 } | if s > line_from { SPACE } else { 0 };
        if unclosed {
            flags |= UNCLOSED_COMMENT;
        }
        (s, flags)
    }

    /// The last `\n` in `from..to` outside comments (a gap's).
    fn last_eol(&self, from: usize, to: usize) -> Option<usize> {
        let mut last = None;
        let mut i = from;
        while let Some(e) = self.lx.blocks.eol_in(i, to) {
            if !self.extras.iter().any(|x| (x.start as usize) <= e && e < x.end as usize) {
                last = Some(e);
            }
            i = e + 1;
        }
        last
    }

    /// The token at `s` (a token's start, a gap passed), with `flags`.
    fn token_at(&self, s: usize, flags: u8) -> PpTok {
        let len = self.lx.src.len();
        let mk = |kind: P, punct: T, end: usize, more: u8| PpTok { kind, punct, flags: flags | more, start: s as u32, end: end as u32 };
        if s >= len {
            return mk(P::Eof, T::Eof, s, 0);
        }
        let (c, d) = (self.at(s), self.at(s + 1));
        if c.is_ascii_digit() || (c == b'.' && d.is_ascii_digit()) {
            return mk(P::Number, T::Eof, self.pp_number_end(s), 0);
        }
        match (c, d) {
            (b'#', b'#') => return mk(P::HashHash, T::Eof, s + 2, 0),
            (b'#', _) => return mk(P::Hash, T::Eof, s + 1, 0),
            (b'%', b':') => {
                return if self.at(s + 2) == b'%' && self.at(s + 3) == b':' { mk(P::HashHash, T::Eof, s + 4, DIGRAPH) } else { mk(P::Hash, T::Eof, s + 2, DIGRAPH) };
            }
            // C++'s `<::`: `<` then `::`, unless `:` or `>` follows ([lex.pptoken] 3.2).
            (b'<', b':') if self.opts.cpp && self.at(s + 2) == b':' && !matches!(self.at(s + 3), b':' | b'>') => {
                return mk(P::Punct, T::Lt, s + 1, 0);
            }
            (b'<', b':') => return mk(P::Punct, T::LBracket, s + 2, DIGRAPH),
            (b':', b'>') => return mk(P::Punct, T::RBracket, s + 2, DIGRAPH),
            (b'<', b'%') => return mk(P::Punct, T::LBrace, s + 2, DIGRAPH),
            (b'%', b'>') => return mk(P::Punct, T::RBrace, s + 2, DIGRAPH),
            (b':', b':') if self.opts.colons => return mk(P::Punct, T::ColonColon, s + 2, 0),
            (b'"', _) => return self.quoted(s, s, P::Str, flags),
            (b'\'', _) => return self.quoted(s, s, P::Char, flags),
            _ => {}
        }
        if c.is_ascii_alphabetic() || c == b'_' || c == b'$' || c >= 0x80 || (c == b'\\' && matches!(d, b'u' | b'U')) {
            if let Some(t) = self.prefixed(s, flags) {
                return t;
            }
            return match self.ident_end(s, true) {
                Some(e) => mk(P::Ident, T::Eof, e, 0),
                None => mk(P::Other, T::Eof, s + utf8_len(c).min(len - s), 0),
            };
        }
        match self.lx.punct(s) {
            Some((t, e)) => mk(P::Punct, t, e, 0),
            None => mk(P::Other, T::Eof, s + 1, 0),
        }
    }

    /// An identifier's end from `s` (ASCII word bytes, `$`, characters past ASCII by their class,
    /// and `\u` and `\U` escapes); `None` where none starts. `first`: `s` is the identifier's first
    /// character (XID_Start), not a continuation's.
    fn ident_end(&self, s: usize, first: bool) -> Option<usize> {
        let src = self.lx.src;
        let mut i = s;
        loop {
            i = self.lx.blocks.word_end(i);
            let c = self.at(i);
            if c == b'\\' && matches!(self.at(i + 1), b'u' | b'U') {
                let n = if self.at(i + 1) == b'u' { 4 } else { 8 };
                if !(2..2 + n).all(|k| self.at(i + k).is_ascii_hexdigit()) {
                    break;
                }
                i += 2 + n;
            } else if c >= 0x80 && i < src.len() {
                let Some((ch, n)) = crate::lex::decode(src, i) else { break };
                let ok = if i == s && first { unicode_ident::is_xid_start(ch) } else { unicode_ident::is_xid_continue(ch) };
                if !ok {
                    break;
                }
                i += n;
            } else {
                break;
            }
        }
        (i > s).then_some(i)
    }

    /// A literal whose quote is at `q` and prefix starts at `s`: its token, or an `Other` (the quote
    /// alone, `UNCLOSED`) where its line ends first.
    fn quoted(&self, s: usize, q: usize, kind: P, flags: u8) -> PpTok {
        let close = self.at(q);
        let mut i = q + 1;
        let end = loop {
            i = rerail_simd8::micro::first_of(self.lx.src, i, [close, b'\\', b'\n']);
            match self.lx.src.get(i) {
                Some(&c) if c == close => break Some(i + 1),
                Some(b'\\') => i += 2,
                _ => break None,
            }
        };
        match end {
            Some(e) => PpTok { kind, punct: T::Eof, flags, start: s as u32, end: self.suffix_end(e) as u32 },
            None => PpTok { kind: P::Other, punct: T::Eof, flags: flags | UNCLOSED, start: s as u32, end: q as u32 + 1 },
        }
    }

    /// A string or character literal whose prefix (`L`, `u`, `U`, `u8`, then `R` where raw strings
    /// are read) starts at `s`; `None` where an identifier starts there.
    fn prefixed(&self, s: usize, flags: u8) -> Option<PpTok> {
        let mut k = s;
        match self.at(k) {
            b'L' | b'U' => k += 1,
            b'u' => k += if self.at(k + 1) == b'8' { 2 } else { 1 },
            _ => {}
        }
        let raw = self.opts.raw_strings && self.at(k) == b'R';
        if raw {
            k += 1;
        }
        let q = self.at(k);
        if k == s || !(q == b'"' || (q == b'\'' && !raw)) {
            return None;
        }
        if raw {
            return Some(match self.lx.raw_end(k + 1) {
                Some(e) => PpTok { kind: P::RawStr, punct: T::Eof, flags, start: s as u32, end: self.suffix_end(e) as u32 },
                // An unclosed raw string: its prefix is an identifier.
                None => return None,
            });
        }
        Some(self.quoted(s, k, if q == b'"' { P::Str } else { P::Char }, flags))
    }

    /// The end of a literal's user-defined suffix from `e` (C++), or `e`.
    fn suffix_end(&self, e: usize) -> usize {
        if !self.opts.cpp {
            return e;
        }
        let c = self.at(e);
        if c.is_ascii_alphabetic() || c == b'_' || c >= 0x80 {
            self.ident_end(e, true).unwrap_or(e)
        } else {
            e
        }
    }

    /// A pp-number's end from `s`.
    fn pp_number_end(&self, s: usize) -> usize {
        let mut i = s + 1;
        loop {
            i = self.lx.blocks.word_end(i);
            let c = self.at(i);
            let prev = self.at(i - 1);
            match c {
                b'.' => i += 1,
                b'+' | b'-' if matches!(prev, b'e' | b'E' | b'p' | b'P') => i += 1,
                b'\'' if self.opts.digit_separators && (self.at(i + 1).is_ascii_alphanumeric() || self.at(i + 1) == b'_') => i += 2,
                b'$' => i += 1,
                b'\\' if matches!(self.at(i + 1), b'u' | b'U') => match self.ident_end(i, false) {
                    Some(e) => i = e,
                    None => return i,
                },
                c if c >= 0x80 => match self.ident_end(i, false) {
                    Some(e) if e > i => i = e,
                    _ => return i,
                },
                _ => return i,
            }
        }
    }

    /// The header name from `pos` (an include's operand): `<…>` or `"…"` on the line, with no
    /// escapes read, or `None` (the operand is tokens, read by `next`).
    pub fn header_name(&mut self) -> Option<PpTok> {
        let save = (self.pos, self.bol);
        let (s, flags) = self.gap();
        if flags & BOL != 0 && !save.1 {
            self.pos = save.0;
            return None;
        }
        let close = match self.at(s) {
            b'<' => b'>',
            b'"' => b'"',
            _ => {
                (self.pos, self.bol) = save;
                return None;
            }
        };
        let mut i = s + 1;
        while i < self.lx.src.len() && self.lx.src[i] != close && self.lx.src[i] != b'\n' {
            i += 1;
        }
        if self.at(i) != close {
            (self.pos, self.bol) = save;
            return None;
        }
        self.pos = i + 1;
        self.bol = false;
        Some(PpTok { kind: P::Header, punct: T::Eof, flags, start: s as u32, end: i as u32 + 1 })
    }

    /// The rest of the current line from `pos`, blanks before it passed: its bytes (an `#error`'s
    /// text, a `#pragma`'s), and the next line's start.
    pub fn rest_of_line(&mut self) -> (usize, usize) {
        let mut s = self.pos;
        while matches!(self.at(s), b' ' | b'\t' | 0x0b | 0x0c | b'\r') {
            s += 1;
        }
        let e = self.lx.blocks.eol(s);
        let mut end = e;
        while end > s && matches!(self.at(end - 1), b' ' | b'\t' | b'\r') {
            end -= 1;
        }
        self.pos = e;
        (s, end)
    }

    /// Passes every token to the end of the current line (a skipped group's line, or a directive's
    /// rest), reading tokens only for their quotes and comments.
    pub fn skip_line(&mut self) {
        loop {
            let save = (self.pos, self.bol);
            let t = self.next();
            if t.kind == P::Eof || (t.flags & BOL != 0) {
                (self.pos, self.bol) = save;
                // The line end ends the line: the next token is the next line's first.
                self.bol = true;
                return;
            }
        }
    }

    /// Whether the next token starts a new line (or the text ends), without reading it.
    pub fn at_line_end(&mut self) -> bool {
        let save = (self.pos, self.bol);
        let t = self.next();
        (self.pos, self.bol) = save;
        t.kind == P::Eof || t.flags & BOL != 0
    }

    /// The next token where it is on the current line; `None` (nothing read) where the line or the
    /// text ends first.
    pub fn next_on_line(&mut self) -> Option<PpTok> {
        let save = (self.pos, self.bol);
        let t = self.next();
        if t.kind == P::Eof || t.flags & BOL != 0 {
            (self.pos, self.bol) = save;
            return None;
        }
        Some(t)
    }

    /// Where the lexer is: its position, and whether no token was read yet on its line (`seek`'s).
    pub fn state(&self) -> (usize, bool) {
        (self.pos, self.bol)
    }

    /// Reads from `at`, as the first token of a line when `bol`.
    pub fn seek(&mut self, at: usize, bol: bool) {
        self.pos = at;
        self.bol = bol;
    }
}

/// The length of a UTF-8 character whose first byte is `c` (1 for an ASCII byte or a stray one).
fn utf8_len(c: u8) -> usize {
    match c {
        0xf0..=0xf7 => 4,
        0xe0..=0xef => 3,
        0xc0..=0xdf => 2,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tokens of `src`, as (kind, punct, flags, text).
    fn toks(src: &str, opts: Options) -> Vec<(P, T, u8, String)> {
        let p2 = Phase2::of(src.as_bytes(), opts.trigraphs);
        let mut lx = PpLexer::new(&p2.text, opts);
        let mut out = Vec::new();
        loop {
            let t = lx.next();
            if t.kind == P::Eof {
                break;
            }
            out.push((t.kind, t.punct, t.flags, String::from_utf8_lossy(&p2.text[t.start as usize..t.end as usize]).into_owned()));
        }
        out
    }

    fn texts(src: &str, opts: Options) -> Vec<String> {
        toks(src, opts).into_iter().map(|t| t.3).collect()
    }

    #[test]
    fn pp_numbers_are_the_standards() {
        let c = Options::c(17, false);
        assert_eq!(texts("0x1e+1 1.2.3 .5e-x 1e 12_ab 0xp+3", c), ["0x1e+1", "1.2.3", ".5e-x", "1e", "12_ab", "0xp+3"]);
        // `+` after a letter other than e or p ends the number.
        assert_eq!(texts("0x1f+1", c), ["0x1f", "+", "1"]);
        // Digit separators where the dialect has them; elsewhere `'` starts a character literal.
        assert_eq!(texts("1'000'000", Options::c(23, false)), ["1'000'000"]);
        assert_eq!(texts("1'2'", c), ["1", "'2'"]);
    }

    #[test]
    fn hashes_digraphs_and_cpp_rules() {
        let c = Options::c(17, false);
        let t = toks("# ## %: %:%: <: :> <% %>", c);
        let kinds: Vec<_> = t.iter().map(|t| (t.0, t.1)).collect();
        assert_eq!(kinds, [(P::Hash, T::Eof), (P::HashHash, T::Eof), (P::Hash, T::Eof), (P::HashHash, T::Eof), (P::Punct, T::LBracket), (P::Punct, T::RBracket), (P::Punct, T::LBrace), (P::Punct, T::RBrace)]);
        assert!(t[2..].iter().all(|t| t.2 & DIGRAPH != 0));
        let cpp = Options::cpp(20, false);
        assert_eq!(texts("a<::b> x<:::y", cpp), ["a", "<", "::", "b", ">", "x", "<:", "::", "y"]);
        assert_eq!(texts("\"s\"_x 'c'_y R\"d(a)\")d\"_z", cpp), ["\"s\"_x", "'c'_y", "R\"d(a)\")d\"_z"]);
        // In C, a suffix is an identifier of its own, and `R"` is an identifier and a string.
        assert_eq!(texts("\"s\"_x R\"(a)\"", c), ["\"s\"", "_x", "R", "\"(a)\""]);
        // `::` is one punctuator in C23 (for attributes), two before.
        assert_eq!(texts("[[gnu::x]]", Options::c(23, false)), ["[", "[", "gnu", "::", "x", "]", "]"]);
        assert_eq!(texts("a::b", c), ["a", ":", ":", "b"]);
    }

    #[test]
    fn nothing_is_refused() {
        let c = Options::c(17, false);
        let t = toks("don't @ ` \\ \"open\n x", c);
        let got: Vec<_> = t.iter().map(|t| (t.0, t.3.as_str())).collect();
        assert_eq!(got, [(P::Ident, "don"), (P::Other, "'"), (P::Ident, "t"), (P::Other, "@"), (P::Other, "`"), (P::Other, "\\"), (P::Other, "\""), (P::Ident, "open"), (P::Ident, "x")]);
        assert!(t[1].2 & UNCLOSED != 0);
        let open = toks("a /* no end", c);
        assert_eq!(open.len(), 1);
    }

    #[test]
    fn lines_and_spaces() {
        let c = Options::c(17, false);
        let t = toks("a b\n  #define x/**/y\nz", c);
        let got: Vec<_> = t.iter().map(|t| (t.3.as_str(), t.2 & (BOL | SPACE))).collect();
        assert_eq!(got, [("a", BOL), ("b", SPACE), ("#", BOL | SPACE), ("define", 0), ("x", SPACE), ("y", SPACE), ("z", BOL)]);
    }

    #[test]
    fn splices_and_trigraphs_are_phases_1_and_2() {
        let c = Options::c(17, false);
        let src = "#def\\\nine A 1\\  \n2\n??=x ??/\ny";
        let p2 = Phase2::of(src.as_bytes(), true);
        assert_eq!(&*p2.text, b"#define A 12\n#x y");
        assert_eq!(texts(src, c), ["#", "define", "A", "12", "#", "x", "y"]);
        // Positions map back to the file's.
        let at = p2.text.iter().position(|&b| b == b'A').unwrap();
        assert_eq!(src.as_bytes()[p2.orig(at)], b'A');
        let y = p2.text.len() - 1;
        assert_eq!(src.as_bytes()[p2.orig(y)], b'y');
        // No trigraphs where the dialect does not read them; a file with neither is read in place.
        assert_eq!(&*Phase2::of(b"??=x", false).text, b"??=x");
        assert!(Phase2::of(b"int x;\n", true).in_place());
        // A byte order mark is passed; positions after it are the file's.
        let p = Phase2::of(b"\xef\xbb\xbf#x", false);
        assert_eq!(&*p.text, b"#x");
        assert_eq!(p.orig(1), 4);
    }

    #[test]
    fn header_names() {
        let c = Options::c(17, false);
        let src = b"include <a/b.h> \"c\\d.h\" x\n";
        let mut lx = PpLexer::new(src, c);
        assert_eq!(lx.next().kind, P::Ident);
        let h = lx.header_name().unwrap();
        assert_eq!(&src[h.start as usize..h.end as usize], b"<a/b.h>");
        let h = lx.header_name().unwrap();
        assert_eq!(&src[h.start as usize..h.end as usize], b"\"c\\d.h\"");
        assert!(lx.header_name().is_none());
        assert_eq!(lx.next().kind, P::Ident);
    }

    /// Every token by `next` against a byte-at-a-time reading of the standard's rules, over random
    /// texts of the pieces.
    #[test]
    fn next_reads_as_the_scalar_rules_do() {
        let pieces = ["a", "_b1", "$x", "L", "u8", "R", "0x1e+", ".5", "1'2", "\"s\\\"t\"", "'c'", "\"open", "'", "#", "##", "%:", "<:", ":>", "<::", "->*", "<<=", "...", ".", "+", "-", "/", "/* c */", "// l", " ", "\t", "\n", "@", "é", "\\u00e9", "R\"d(x)d\""];
        let mut x = 0x2545_f491_4f6c_dd1du64;
        for opts in [Options::c(17, false), Options::c(23, true), Options::cpp(20, false)] {
            for _ in 0..300 {
                let mut s = String::new();
                for _ in 0..12 {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    s.push_str(pieces[(x % pieces.len() as u64) as usize]);
                }
                let got = texts(&s, opts);
                let want = scalar(s.as_bytes(), opts);
                assert_eq!(got, want, "{s:?} {opts:?}");
            }
        }
    }

    /// The reference: one byte at a time.
    fn scalar(src: &[u8], o: Options) -> Vec<String> {
        let at = |i: usize| src.get(i).copied().unwrap_or(0);
        let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'$';
        let mut out = Vec::new();
        let mut i = 0;
        while i < src.len() {
            let c = at(i);
            if matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
                i += 1;
                continue;
            }
            if c == b'/' && at(i + 1) == b'*' {
                let mut k = i + 2;
                while k < src.len() && !(at(k) == b'*' && at(k + 1) == b'/') {
                    k += 1;
                }
                i = (k + 2).min(src.len());
                continue;
            }
            if c == b'/' && at(i + 1) == b'/' {
                while i < src.len() && at(i) != b'\n' {
                    i += 1;
                }
                continue;
            }
            let s = i;
            let quoted = |q: usize| -> Option<usize> {
                let close = at(q);
                let mut k = q + 1;
                loop {
                    match at(k) {
                        _ if k >= src.len() => return None,
                        b'\n' => return None,
                        b'\\' => k += 2,
                        ch if ch == close => return Some(k + 1),
                        _ => k += 1,
                    }
                }
            };
            let ident = |mut k: usize| -> usize {
                loop {
                    let ch = at(k);
                    if word(ch) {
                        k += 1;
                    } else if ch == b'\\' && at(k + 1) == b'u' && (2..6).all(|j| at(k + j).is_ascii_hexdigit()) {
                        k += 6;
                    } else if ch >= 0x80 {
                        let n = utf8_len(ch);
                        match std::str::from_utf8(&src[k..(k + n).min(src.len())]).ok().and_then(|t| t.chars().next()) {
                            Some(chr) if (k == s && unicode_ident::is_xid_start(chr)) || (k > s && unicode_ident::is_xid_continue(chr)) => k += n,
                            _ => return k,
                        }
                    } else {
                        return k;
                    }
                }
            };
            let suffix = |e: usize| if o.cpp && (at(e).is_ascii_alphabetic() || at(e) == b'_' || at(e) >= 0x80) { ident(e).max(e) } else { e };
            let end = if c.is_ascii_digit() || (c == b'.' && at(i + 1).is_ascii_digit()) {
                let mut k = i + 1;
                loop {
                    let ch = at(k);
                    if word(ch) || ch == b'.' {
                        k += 1;
                    } else if matches!(ch, b'+' | b'-') && matches!(at(k - 1), b'e' | b'E' | b'p' | b'P') {
                        k += 1;
                    } else if ch == b'\'' && o.digit_separators && (at(k + 1).is_ascii_alphanumeric() || at(k + 1) == b'_') {
                        k += 2;
                    } else if ch == b'\\' && at(k + 1) == b'u' && (2..6).all(|j| at(k + j).is_ascii_hexdigit()) {
                        k += 6;
                    } else if ch >= 0x80 {
                        let e = ident(k);
                        if e == k {
                            break;
                        }
                        k = e;
                    } else {
                        break;
                    }
                }
                k
            } else if c == b'"' || c == b'\'' {
                quoted(i).map_or(i + 1, suffix)
            } else if c.is_ascii_alphabetic() || c == b'_' || c == b'$' || c >= 0x80 || (c == b'\\' && at(i + 1) == b'u') {
                let mut k = i;
                match at(k) {
                    b'L' | b'U' => k += 1,
                    b'u' => k += if at(k + 1) == b'8' { 2 } else { 1 },
                    _ => {}
                }
                let raw = o.raw_strings && at(k) == b'R';
                if raw {
                    k += 1;
                }
                let lit = if k > i && at(k) == b'"' && raw {
                    // `R"d(…)d"`: the delimiter, then its close.
                    let d0 = k + 1;
                    let mut p = d0;
                    while p < src.len() && !matches!(at(p), b'(' | b')' | b'\\' | b' ' | b'\t' | b'\n' | b'"') && p - d0 <= 16 {
                        p += 1;
                    }
                    if at(p) == b'(' {
                        let delim = &src[d0..p];
                        let mut q = p + 1;
                        let mut found = None;
                        while q < src.len() {
                            if at(q) == b')' && src.get(q + 1..q + 1 + delim.len()) == Some(delim) && at(q + 1 + delim.len()) == b'"' {
                                found = Some(q + 2 + delim.len());
                                break;
                            }
                            q += 1;
                        }
                        found.map(suffix)
                    } else {
                        None
                    }
                } else if k > i && (at(k) == b'"' || at(k) == b'\'') && !raw {
                    Some(quoted(k).map_or(k + 1, suffix))
                } else {
                    None
                };
                match lit {
                    Some(e) => e,
                    None => {
                        let e = ident(i);
                        if e > i { e } else { i + utf8_len(c).min(src.len() - i) }
                    }
                }
            } else {
                let three = [b"<<=" as &[u8], b">>=", b"...", b"->*", b"<=>", b"%:%:"];
                let two = [b"##" as &[u8], b"%:", b"<:", b":>", b"<%", b"%>", b"->", b"++", b"--", b"<<", b">>", b"<=", b">=", b"==", b"!=", b"&&", b"||", b"*=", b"/=", b"%=", b"+=", b"-=", b"&=", b"^=", b"|=", b"::", b".*"];
                let colons = |t: &[u8]| t != b"::" || o.colons;
                let rest = &src[i..];
                if o.cpp && rest.starts_with(b"<::") && !matches!(at(i + 3), b':' | b'>') {
                    i + 1
                } else if let Some(t) = three.iter().find(|t| rest.starts_with(t) && (o.cpp || !matches!(**t, b"->*" | b"<=>"))) {
                    // `%:%:` is four bytes.
                    i + t.len()
                } else if let Some(t) = two.iter().find(|t| rest.starts_with(t) && colons(t) && (o.cpp || **t != *b".*")) {
                    i + t.len()
                } else if b"()[]{};,~?:.=!+-*/%^&|<>#".contains(&c) {
                    i + 1
                } else {
                    i + 1
                }
            };
            out.push(String::from_utf8_lossy(&src[s..end]).into_owned());
            i = end;
        }
        out
    }
}
