//! The text's byte classes, 64 bytes a step, as simdjson's two stages split the work (as in
//! `rerail-java-lean`):
//!
//! - **Stage 1** (`Blocks::of`) reads the whole text once, a 64-byte block a step, and keeps a mask a
//!   class, one bit a byte: a name's bytes, blanks, line ends, and the bytes that may end a string's
//!   or a block comment's run. The classes come
//!   from simdjson's nibble tables (`LO`, `HI`), the rest from compares, written once over
//!   `rerail_simd8`'s operations and compiled for each backend (NEON, AVX2, SSE, the fallback).
//! - **Stage 2** is the lexer: a run's end, or the next byte of a class, is the lowest bit of a
//!   block's mask (`trailing_zeros`), the bits below the start cleared.
//!
//! Past the text's end the runs (names, blanks) stop and the stops are all set, so a scan never
//! reads past the last block; its result is at most the text's length.

use rerail_simd8::Simd8x64;

/// What a block lexer's reader made of a token start (`Blocks::lex_blocks`).
pub enum Lexed<Tk> {
    /// A token, and its end.
    Tok(Tk, usize),
    /// A comment (written to the extras), and its end.
    Extra(usize),
    /// Nothing: the lexer's token reader reads it.
    Slow,
}

/// A block's masks, by index.
const WORD: usize = 0; // [A-Za-z0-9_]
const BLANK: usize = 1; // ' ', '\t', '\v', '\f'
const EOL: usize = 2; // '\n', '\r'
const STOP: usize = 3; // a string's: '"', '\\', '$'
const STAR: usize = 4; // a block comment's: '*', '/'
const MASKS: usize = 5;

// The nibble tables' bits: each is the product of a set of high nibbles and a set of low ones.
const C_AO: u8 = 1; // high 4 or 6, low 1..=15: A-O, a-o
const C_PZ: u8 = 2; // high 5 or 7, low 0..=10: P-Z, p-z
const C_DIGIT: u8 = 4; // high 3, low 0..=9
const C_UNDERSCORE: u8 = 8; // high 5, low 15
const C_SPACE: u8 = 16; // high 2, low 0
const C_TAB_VT_FF: u8 = 32; // high 0, low 9, 11 or 12
const C_EOL: u8 = 64; // high 0, low 10 or 13
const C_QUOTE: u8 = 128; // high 2, low 2 or 7: '"', '\''
const C_WORD: u8 = C_AO | C_PZ | C_DIGIT | C_UNDERSCORE;
const C_BLANK: u8 = C_SPACE | C_TAB_VT_FF;

static LO: [u8; 16] = {
    let mut t = [0u8; 16];
    let mut n = 0;
    while n < 16 {
        let mut b = 0;
        if n >= 1 {
            b |= C_AO;
        }
        if n <= 10 {
            b |= C_PZ;
        }
        if n <= 9 {
            b |= C_DIGIT;
        }
        if n == 15 {
            b |= C_UNDERSCORE;
        }
        if n == 0 {
            b |= C_SPACE;
        }
        if n == 9 || n == 11 || n == 12 {
            b |= C_TAB_VT_FF;
        }
        if n == 10 || n == 13 {
            b |= C_EOL;
        }
        if n == 2 || n == 7 {
            b |= C_QUOTE;
        }
        t[n] = b;
        n += 1;
    }
    t
};

static HI: [u8; 16] = {
    let mut t = [0u8; 16];
    t[0] = C_TAB_VT_FF | C_EOL;
    t[2] = C_SPACE | C_QUOTE;
    t[3] = C_DIGIT;
    t[4] = C_AO;
    t[5] = C_PZ | C_UNDERSCORE;
    t[6] = C_AO;
    t[7] = C_PZ;
    t
};

/// A byte's classes, by the same tables.
#[inline(always)]
fn class(c: u8) -> u8 {
    LO[usize::from(c & 15)] & HI[usize::from(c >> 4)]
}

/// Whether `c` is white space (a blank or a line end). (Swift's lexer, sharing this file, has no
/// use for it: its fast exit measured slower.)
#[allow(dead_code)]
#[inline(always)]
pub fn is_space(c: u8) -> bool {
    class(c) & (C_BLANK | C_EOL) != 0
}

/// Whether `c` may continue a name (ASCII only: a byte from 0x80 is read as a character).
#[inline(always)]
pub fn is_word(c: u8) -> bool {
    class(c) & C_WORD != 0
}

type Masks = [u64; MASKS];

/// One block's masks, and its NULs' mask.
#[inline(always)]
unsafe fn masks<S: Simd8x64>(b: &[u8; 64]) -> (Masks, u64) {
    unsafe {
        let v = S::load(b);
        let c = S::classify(v, &LO, &HI);
        let is = |x: u8| S::eq(v, S::splat(x));
        let eol = S::to_bitmask(S::test(c, S::splat(C_EOL)));
        let word = S::to_bitmask(S::test(c, S::splat(C_WORD)));
        let blank = S::to_bitmask(S::test(c, S::splat(C_BLANK)));
        let stop = S::to_bitmask(S::or(S::or(is(b'"'), is(b'\\')), is(b'$')));
        let star = S::to_bitmask(S::or(is(b'*'), is(b'/')));
        ([word, blank, eol, stop, star], S::to_bitmask(is(0)))
    }
}

/// Stage 1 over `src`: each block's masks, the last block's padded, and its first NUL's offset.
#[inline(always)]
unsafe fn stage1<S: Simd8x64>(src: &[u8]) -> (Vec<Masks>, Option<usize>) {
    let mut out = Vec::with_capacity(src.len().div_ceil(64));
    let mut nul = None;
    let mut at = 0;
    while let Some(b) = src.get(at..at + 64) {
        let (m, z) = unsafe { masks::<S>(b.try_into().expect("64 bytes")) };
        out.push(m);
        if z != 0 && nul.is_none() {
            nul = Some(at + z.trailing_zeros() as usize);
        }
        at += 64;
    }
    if at < src.len() {
        let mut b = [0u8; 64];
        let n = src.len() - at;
        b[..n].copy_from_slice(&src[at..]);
        let (mut m, z) = unsafe { masks::<S>(&b) };
        let past = !0u64 << n;
        if z & !past != 0 && nul.is_none() {
            nul = Some(at + z.trailing_zeros() as usize);
        }
        m[WORD] &= !past;
        m[BLANK] &= !past;
        for k in EOL..MASKS {
            m[k] |= past;
        }
        out.push(m);
    }
    (out, nul)
}

rerail_simd8::dispatch! {
    fn stage1_best(src: &[u8]) -> (Vec<Masks>, Option<usize>) = stage1, on stage1_on;
}

/// The text's masks (stage 1), for the lexer's scans (stage 2).
pub struct Blocks<'s> {
    src: &'s [u8],
    m: Vec<Masks>,
    nul: Option<usize>,
}

impl<'s> Blocks<'s> {
    /// Stage 1 over `src`, on the best backend the CPU runs.
    pub fn of(src: &'s [u8]) -> Self {
        let (m, nul) = stage1_best(src);
        Blocks { src, m, nul }
    }

    /// The text's first NUL, if it holds one (a syntax error in every token and between them).
    pub fn first_nul(&self) -> Option<usize> {
        self.nul
    }

    /// The first byte from `at` on whose bit in mask `k` is `set`.
    #[inline(always)]
    fn next(&self, at: usize, k: usize, set: bool) -> usize {
        if at >= self.src.len() {
            return self.src.len();
        }
        let flip = if set { 0 } else { !0 };
        let mut b = at / 64;
        let mut m = (self.m[b][k] ^ flip) & (!0u64 << (at % 64));
        loop {
            if m != 0 {
                return (b * 64 + m.trailing_zeros() as usize).min(self.src.len());
            }
            b += 1;
            if b >= self.m.len() {
                return self.src.len();
            }
            m = self.m[b][k] ^ flip;
        }
    }

    /// Tokens from `at` on, a block at a time (docs/parser-generators/31, the block lexer): a
    /// block's token starts found at once (`!space & !(word & word << 1)`), each read by `fast`
    /// from its start, given the length of the word run there when it ends inside the block, and
    /// whether a line end is in the white space before it (both readers given `cx`, the comments'
    /// list). `fast` makes a token and its end, or
    /// reads a comment (its end), or leaves the start to `slow`: the lexer's token reader from the
    /// end of the last token or comment (characters past ASCII), given whether a line end was
    /// passed since the last token (before comments), which gives a token, its end and whether it
    /// is the text's end. Stops at a block's end once `min` tokens are made, or at the text's end;
    /// returns where the next token is to be read from.
    #[inline(always)]
    pub fn lex_blocks<Tk, E, C>(
        &self,
        at: usize,
        out: &mut Vec<Tk>,
        min: usize,
        cx: &mut C,
        fast: impl Fn(&mut C, usize, usize, bool) -> Result<Lexed<Tk>, E>,
        slow: impl Fn(&mut C, usize, bool) -> Result<(Tk, usize, bool), E>,
    ) -> Result<usize, E> {
        let len = self.src.len();
        let made = out.len();
        // The end of the last token or comment, and whether a line end is between the last token
        // and the block read (outside comments).
        let (mut gap, mut carry) = (at, false);
        // Whether comments were read since the last token (a stop then would lose `carry`).
        let mut extra = false;
        let mut b = at / 64;
        'blocks: loop {
            if b >= self.m.len() {
                let (t, e, end) = slow(cx, gap, carry)?;
                out.push(t);
                if end {
                    return Ok(e);
                }
                gap = e;
                carry = false;
                extra = false;
                b = e / 64;
                continue;
            }
            let base = b * 64;
            let m = &self.m[b];
            let (word, eol) = (m[WORD], m[EOL]);
            let space = m[BLANK] | eol;
            let from = gap.saturating_sub(base);
            let mut starts = !space & !(word & (word << 1)) & (!0u64 << from);
            // A token starts where the last one ended, inside a word's run too (`0x1Fg`).
            if gap >= base && gap < len && !space >> from & 1 == 1 {
                starts |= 1 << from;
            }
            while starts != 0 {
                let q = starts.trailing_zeros() as usize;
                let g = gap.saturating_sub(base);
                let nl = carry || eol & (!0u64 << g) & ((1u64 << q) - 1) != 0;
                let run = (!(word >> q)).trailing_zeros() as usize;
                let run = if q + run < 64 { run } else { usize::MAX };
                let e = match fast(cx, base + q, run, nl)? {
                    Lexed::Tok(t, e) => {
                        out.push(t);
                        carry = false;
                        extra = false;
                        e
                    }
                    Lexed::Extra(e) => {
                        // The white space before the comment is the next token's.
                        carry = nl;
                        extra = true;
                        e
                    }
                    Lexed::Slow => {
                        let (t, e, end) = slow(cx, gap, carry)?;
                        out.push(t);
                        if end {
                            return Ok(e);
                        }
                        carry = false;
                        extra = false;
                        e
                    }
                };
                gap = e;
                if e >= base + 64 {
                    b = e / 64;
                    continue 'blocks;
                }
                starts &= !0u64 << (e - base);
                if e < len && !space >> (e - base) & 1 == 1 {
                    starts |= 1 << (e - base);
                }
            }
            // The line ends after the last token, up to the text's end.
            let g = gap.saturating_sub(base);
            let real = if base + 64 > len { (1u64 << (len - base)) - 1 } else { !0 };
            carry |= g < 64 && eol & (!0u64 << g) & real != 0;
            b += 1;
            if out.len() - made >= min && !extra {
                return Ok(gap);
            }
        }
    }

    /// The end of a run of `[A-Za-z0-9_$]` from `at`.
    #[inline]
    pub fn word_end(&self, at: usize) -> usize {
        self.next(at, WORD, false)
    }

    /// The end of a run of blanks (`' '`, `'\t'`, `'\v'`, `'\f'`) from `at`.
    #[inline]
    pub fn blank_end(&self, at: usize) -> usize {
        self.next(at, BLANK, false)
    }

    /// The first `\n` or `\r` from `at` (or the text's length): a line comment's end.
    #[inline]
    pub fn eol(&self, at: usize) -> usize {
        self.next(at, EOL, true)
    }

    /// The first `"` or `\` from `at`: where a string's text may end.
    #[inline]
    pub fn string_stop(&self, at: usize) -> usize {
        self.next(at, STOP, true)
    }

    /// The first `*` or `/` from `at`: where a block comment may end.
    #[inline]
    pub fn star(&self, at: usize) -> usize {
        self.next(at, STAR, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The masks one byte at a time, and the first NUL.
    fn scalar(src: &[u8]) -> (Vec<Masks>, Option<usize>) {
        let mut out = vec![[0u64; MASKS]; src.len().div_ceil(64)];
        for (b, o) in out.iter_mut().enumerate() {
            for i in 0..64 {
                let at = b * 64 + i;
                let Some(&c) = src.get(at) else {
                    for m in &mut o[EOL..] {
                        *m |= 1 << i;
                    }
                    continue;
                };
                let bit = |x: bool| u64::from(x) << i;
                o[WORD] |= bit(c.is_ascii_alphanumeric() || c == b'_');
                o[BLANK] |= bit(matches!(c, b' ' | b'\t' | 0x0b | 0x0c));
                o[EOL] |= bit(matches!(c, b'\n' | b'\r'));
                o[STOP] |= bit(matches!(c, b'"' | b'\\' | b'$'));
                o[STAR] |= bit(matches!(c, b'*' | b'/'));
            }
        }
        (out, src.iter().position(|&c| c == 0))
    }

    #[test]
    fn every_backend_gives_the_byte_tests_masks() {
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        let alphabet = b"aZ_9$ \t\x0b\n\r\x0c'\"`\\{}#.(@[]/*<>&\x7f";
        for len in [0usize, 1, 63, 64, 65, 200, 1000] {
            for _ in 0..20 {
                let src: Vec<u8> = (0..len)
                    .map(|_| {
                        x ^= x << 13;
                        x ^= x >> 7;
                        x ^= x << 17;
                        if x % 3 == 0 { (x >> 16) as u8 } else { alphabet[(x >> 8) as usize % alphabet.len()] }
                    })
                    .collect();
                let want = scalar(&src);
                for be in rerail_simd8::Backend::ALL {
                    if let Some(got) = stage1_on(be, &src) {
                        assert_eq!(got, want, "{be:?}, {len} bytes");
                    }
                }
            }
        }
        for c in 0..=255u8 {
            assert_eq!(is_word(c), c.is_ascii_alphanumeric() || c == b'_', "{c}");
        }
    }

    #[test]
    fn runs_end_where_they_should() {
        let s = b"abc_9 \t x";
        let b = Blocks::of(s);
        assert_eq!(b.word_end(0), 5);
        assert_eq!(b.blank_end(5), 8);
        let long = [b'a'; 200];
        assert_eq!(Blocks::of(&long).word_end(3), 200);
        assert_eq!(Blocks::of(b"// x\r\n").eol(0), 4);
        assert_eq!(Blocks::of(b"ab\\c\"").string_stop(0), 2);
        assert_eq!(Blocks::of(b"/* a */").star(2), 5);
        assert_eq!(Blocks::of(b"ab").first_nul(), None);
        assert_eq!(Blocks::of(&[[b'a'; 70].as_slice(), b"\0b\0"].concat()).first_nul(), Some(70));
    }
}
