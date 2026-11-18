//! The C and C++ frontends' input from mode (b) (decision 0327, "The code"): an expanded text, the
//! parser's tokens over it, and where each came from.
//!
//! The text is the translation unit's tokens, a line break where a source line started and a blank
//! where there was white space or where two tokens would otherwise join. The tokens are the ones
//! `rerail_c_lean`'s lexer would make of that text, so `rerail_c_lean::tape_of_tokens` parses them
//! with no lexing. Digraphs are written in their plain spelling (`<:` as `[`); a C++ literal's
//! suffix is a name of its own, as the lexer reads it.
//!
//! **Origins** are run-length segments, not a row a token: a segment starts where the text stops
//! following its file byte for byte (or its expansion changes), and says where its first byte came
//! from: a file position, or an expansion (`Origin::exp`).

use crate::macros::{BOL, DIGRAPH, SPACE, Tk};
use crate::sink::{At, Sink, TextSink};
use rerail_c_lean::pp::P;
use rerail_c_lean::{NO_NL, Refusal, T, Tok, number_end, refused, word};

/// `Origin::exp` of text the preprocessor made outside any expansion (a built-in's value read
/// from a file: `__LINE__`).
pub const MADE: u32 = u32::MAX;

/// A segment of the expanded text: from text position `at` (to the next segment's), the text is
/// file `file`'s from position `off` on, or (`exp != 0`) expansion `exp`'s.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Origin {
    pub at: u32,
    pub file: u32,
    pub off: u32,
    pub exp: u32,
}

/// A translation unit for the frontends.
#[derive(Default)]
pub struct Expanded {
    pub text: Vec<u8>,
    /// The parser's tokens over `text`, ending in `Eof`.
    pub toks: Vec<Tok>,
    pub origins: Vec<Origin>,
}

impl Expanded {
    /// Where text position `at` came from: the segment holding it, and the position within its
    /// file (for a file's segment).
    pub fn origin(&self, at: u32) -> Option<(Origin, u32)> {
        let k = self.origins.partition_point(|o| o.at <= at).checked_sub(1)?;
        let o = self.origins[k];
        Some((o, if o.exp == 0 { o.off + (at - o.at) } else { 0 }))
    }
}

/// The sink that makes an `Expanded`.
pub struct TokenSink {
    pub out: Expanded,
    cpp: bool,
    /// The last token's last byte and kind (`P::Eof`: none yet).
    last: (u8, P),
    /// The first `\n` written since the last token.
    nl: u32,
}

impl TokenSink {
    pub fn new(cpp: bool) -> Self {
        TokenSink { out: Expanded::default(), cpp, last: (0, P::Eof), nl: NO_NL }
    }

    /// The expanded unit, its `Eof` written.
    pub fn finish(mut self) -> Expanded {
        let at = self.out.text.len() as u32;
        self.out.toks.push(Tok { kind: T::Eof, word: 0, start: at, end: at, nl: self.nl });
        self.out
    }

    fn push(&mut self, kind: T, start: usize, end: usize) {
        let w = if kind == T::Name { word(&self.out.text, start, end) } else { 0 };
        self.out.toks.push(Tok { kind, word: w, start: start as u32, end: end as u32, nl: self.nl });
        self.nl = NO_NL;
    }

    /// A token the parser refuses, at text position `at`.
    fn refuse(&mut self, at: usize) {
        self.out.toks.push(Tok { nl: self.nl, ..refused(Refusal::Lex(at as u32)) });
        self.nl = NO_NL;
    }

    /// The segment for a token written at `at`.
    fn origin(&mut self, at: u32, t: &Tk, off: u32) {
        let (file, exp) = match (t.exp, t.buf) {
            (0, b) if b != 0 => (b - 1, 0),
            (0, _) => (0, MADE),
            (e, _) => (0, e),
        };
        if let Some(o) = self.out.origins.last() {
            let follows = if exp == 0 { o.exp == 0 && o.file == file && off.wrapping_sub(o.off) == at - o.at } else { o.exp == exp };
            if follows {
                return;
            }
        }
        self.out.origins.push(Origin { at, file, off: if exp == 0 { off } else { 0 }, exp });
    }
}

/// A digraph's plain spelling.
fn plain(t: T) -> &'static [u8] {
    match t {
        T::LBracket => b"[",
        T::RBracket => b"]",
        T::LBrace => b"{",
        T::RBrace => b"}",
        _ => b"",
    }
}

impl Sink for TokenSink {
    fn token(&mut self, t: &Tk, text: &[u8], _at: Option<At>, off: u32) {
        let text: &[u8] = if t.flags & DIGRAPH != 0 && t.kind == P::Punct { plain(t.punct) } else { text };
        let first = text.first().copied().unwrap_or(0);
        if self.last.1 != P::Eof {
            if t.flags & BOL != 0 {
                if self.nl == NO_NL {
                    self.nl = self.out.text.len() as u32;
                }
                self.out.text.push(b'\n');
            } else if t.flags & SPACE != 0 || TextSink::would_join(self.last, first, t.kind) {
                self.out.text.push(b' ');
            }
        }
        let s = self.out.text.len();
        self.out.text.extend_from_slice(text);
        let e = self.out.text.len();
        self.origin(s as u32, t, off);
        self.last = (text.last().copied().unwrap_or(0), t.kind);
        let lit = match t.kind {
            P::Ident => T::Name,
            P::Number => T::Number,
            P::Char => T::Char,
            P::Str => T::Str,
            P::RawStr => T::RawStr,
            P::Punct => t.punct,
            // `#`, `##` and stray bytes left in the output: the parser refuses them, as it refuses
            // them in a file's text.
            _ => return self.refuse(s),
        };
        // A number as the lexer ends it: a pp-number is longer where a C++ literal's suffix
        // follows (`100ms`: a number and a name), or where the lexer refuses it.
        if lit == T::Number {
            match number_end(text, 0, self.cpp) {
                Ok(k) if k == text.len() => self.push(lit, s, e),
                Ok(k) if self.cpp && text[k..].iter().all(|&c| c.is_ascii_alphanumeric() || c == b'_') && !text[k].is_ascii_digit() => {
                    self.push(lit, s, s + k);
                    self.push(T::Name, s + k, e);
                }
                Ok(k) => self.refuse(s + k),
                Err(Refusal::Lex(at) | Refusal::Parse(at)) => self.refuse(s + at as usize),
                Err(Refusal::TooLarge) => self.refuse(s),
            }
            return;
        }
        // A C++ literal's suffix (`"s"_x`, `'c'_y`) is a name after it.
        if self.cpp && matches!(lit, T::Str | T::Char | T::RawStr) {
            let q = if lit == T::Char { b'\'' } else { b'"' };
            if let Some(k) = text.iter().rposition(|&c| c == q).filter(|&k| k + 1 < text.len()) {
                self.push(lit, s, s + k + 1);
                self.push(T::Name, s + k + 1, e);
                return;
            }
        }
        self.push(lit, s, e);
    }
}

#[cfg(test)]
mod tests;
