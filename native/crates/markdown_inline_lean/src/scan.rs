//! tree-sitter-md-inline's external scanner (`markdown-inline/src/scanner.c`), ported: its state,
//! and its tokens by the symbols the parser's state takes (`Valid`). A scan that finds no token
//! leaves the state as it was (tree-sitter restores it before each scan).

use crate::block::Blocks;
use rerail_simd8::swar::run_end;

/// The external tokens a state takes, a bit each (the scanner's `valid_symbols`).
pub mod x {
    pub const OPEN_STAR: u16 = 1;
    pub const CLOSE_STAR: u16 = 2;
    pub const OPEN_US: u16 = 4;
    pub const CLOSE_US: u16 = 8;
    pub const OPEN_TILDE: u16 = 16;
    pub const CLOSE_TILDE: u16 = 32;
    pub const CODE_START: u16 = 64;
    pub const CODE_CLOSE: u16 = 128;
    pub const LATEX_START: u16 = 256;
    pub const LATEX_CLOSE: u16 = 512;
    pub const UNCLOSED: u16 = 1024;
    /// `_last_token_whitespace`: the last token was white space, a line's end, or none.
    pub const LTW: u16 = 2048;
    /// `_last_token_punctuation`: the last token was punctuation (or an opener).
    pub const LTP: u16 = 4096;
}

/// An emphasis delimiter's character.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum D {
    Star,
    Us,
    Tilde,
}

impl D {
    /// Its opener's and closer's bits in `x`.
    #[inline]
    pub const fn bits(self) -> (u16, u16) {
        match self {
            D::Star => (x::OPEN_STAR, x::CLOSE_STAR),
            D::Us => (x::OPEN_US, x::CLOSE_US),
            D::Tilde => (x::OPEN_TILDE, x::CLOSE_TILDE),
        }
    }
}

/// The scanner's state (`Scanner` in scanner.c): whether the last emphasis delimiter opened, how
/// many of its run are left, and the open code and LaTeX spans' delimiter lengths.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Sc {
    pub open: bool,
    pub left: u8,
    pub code: u8,
    pub latex: u8,
}

/// An external token.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ext {
    Open(D),
    Close(D),
    CodeStart,
    CodeClose,
    LatexStart,
    LatexClose,
    Unclosed,
}

/// Why a scan is refused: a delimiter run of 255 or more (the scanner's `uint8_t` counts wrap).
pub struct LongRun;

#[inline]
fn is_punctuation(c: u8) -> bool {
    c.is_ascii_punctuation()
}

/// The scanner at `at` (its lookahead `src[at]`), with the state's `valid` symbols: the token and its
/// end, the state changed; `None` (the state unchanged) where it finds none.
pub fn scan(src: &[u8], b: &Blocks, at: usize, valid: u16, sc: &mut Sc) -> Result<Option<(Ext, usize)>, LongRun> {
    let mut s = *sc;
    let found = match src[at] {
        b'`' => leaf(src, b, at, valid, &mut s.code, b'`', (x::CODE_START, x::CODE_CLOSE), (Ext::CodeStart, Ext::CodeClose))?,
        b'$' => leaf(src, b, at, valid, &mut s.latex, b'$', (x::LATEX_START, x::LATEX_CLOSE), (Ext::LatexStart, Ext::LatexClose))?,
        b'*' => delimiter(src, at, valid, &mut s, D::Star)?,
        b'_' => delimiter(src, at, valid, &mut s, D::Us)?,
        b'~' => delimiter(src, at, valid, &mut s, D::Tilde)?,
        _ => None,
    };
    if found.is_some() {
        *sc = s;
    }
    Ok(found)
}

/// `parse_leaf_delimiter`: a code (or LaTeX) span's run of `c`, closing the open span of its
/// length, or opening one where a run of the same length follows (`b`'s masks find each run).
#[allow(clippy::too_many_arguments)]
fn leaf(src: &[u8], b: &Blocks, at: usize, valid: u16, len: &mut u8, c: u8, (open_x, close_x): (u16, u16), (open, close): (Ext, Ext)) -> Result<Option<(Ext, usize)>, LongRun> {
    let end = run_end(src, at, c);
    let level = end - at;
    if level >= 255 {
        return Err(LongRun);
    }
    if level == usize::from(*len) && valid & close_x != 0 {
        *len = 0;
        return Ok(Some((close, end)));
    }
    if valid & open_x != 0 {
        // A later run of exactly `level`: each run's start and end from the masks.
        let mut from = end;
        loop {
            let s = if c == b'`' { b.next_tick(from) } else { b.next_dollar(from) };
            if s >= src.len() {
                break;
            }
            let e = if c == b'`' { b.tick_end(s) } else { b.dollar_end(s) };
            if e - s == level {
                *len = level as u8;
                return Ok(Some((open, end)));
            }
            from = e;
        }
        if valid & x::UNCLOSED != 0 {
            return Ok(Some((Ext::Unclosed, end)));
        }
    }
    Ok(None)
}

/// `parse_star`, `parse_underscore`, `parse_tilde`: an emphasis delimiter, one character a token,
/// opening or closing by the run it is in and the characters around it.
fn delimiter(src: &[u8], at: usize, valid: u16, s: &mut Sc, d: D) -> Result<Option<(Ext, usize)>, LongRun> {
    let (open_x, close_x) = d.bits();
    if s.left > 0 {
        if s.open && valid & open_x != 0 {
            s.open = false;
            s.left -= 1;
            return Ok(Some((Ext::Open(d), at + 1)));
        }
        if valid & close_x != 0 {
            s.left -= 1;
            return Ok(Some((Ext::Close(d), at + 1)));
        }
    }
    let c = src[at];
    let end = run_end(src, at, c);
    let count = end - at;
    if count >= 255 {
        return Err(LongRun);
    }
    let next = src.get(end).copied();
    let line_end = matches!(next, None | Some(b'\n' | b'\r'));
    if valid & (open_x | close_x) != 0 {
        s.left = (count - 1) as u8;
        let next_ws = line_end || matches!(next, Some(b' ' | b'\t'));
        let next_punct = next.is_some_and(is_punctuation);
        if valid & close_x != 0 && valid & x::LTW == 0 && (valid & x::LTP == 0 || next_punct || next_ws) {
            s.open = false;
            return Ok(Some((Ext::Close(d), at + 1)));
        }
        if !next_ws && (!next_punct || valid & (x::LTP | x::LTW) != 0) {
            s.open = true;
            return Ok(Some((Ext::Open(d), at + 1)));
        }
    }
    Ok(None)
}
