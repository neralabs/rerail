//! The text's byte classes, 64 bytes a step (stage 1, as in `rerail-css-lean`): a mask a class,
//! one bit a byte, from simdjson's nibble tables (`LO`, `HI`) and compares over `rerail_simd8`'s
//! operations, compiled for each backend. The lexer (stage 2) takes a run's end, or the next byte
//! of a class, as the lowest bit of a block's mask.
//!
//! Past the text's end the runs stop and the stops are all set, so a scan never reads past the last
//! block; its result is at most the text's length.

use rerail_simd8::Simd8x64;

/// A block's masks, by index.
const BLANK: usize = 0; // ASCII white space: ' ', '\t', '\n', '\v', '\f', '\r'
const WORD: usize = 1; // a word's bytes: [A-Za-z0-9_$], and any byte from 0x80 on
const SLASH_STAR: usize = 2; // '*', '/': where a comment may end, or a nested one start
const LF: usize = 3; // '\n': a line comment's end
const SQ: usize = 4; // a single-quoted string's stops: '\'', '\\'
const DQ: usize = 5; // a double-quoted identifier's stop: '"'
const NUL: usize = 6; // a NUL, which the parser does not read
const MASKS: usize = 7;

// The nibble tables' bits: each is the product of a set of high nibbles and a set of low ones.
const C_AO: u8 = 1; // high 4 or 6, low 1..=15: A-O, a-o
const C_PZ: u8 = 2; // high 5 or 7, low 0..=10: P-Z, p-z
const C_DIGIT: u8 = 4; // high 3, low 0..=9
const C_DOLLAR: u8 = 8; // high 2, low 4
const C_UNDER: u8 = 16; // high 5, low 15
const C_SPACE: u8 = 32; // high 2, low 0
const C_CONTROL: u8 = 64; // high 0, low 9..=13
const C_WORD: u8 = C_AO | C_PZ | C_DIGIT | C_DOLLAR | C_UNDER;
const C_BLANK: u8 = C_SPACE | C_CONTROL;

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
        if n == 4 {
            b |= C_DOLLAR;
        }
        if n == 15 {
            b |= C_UNDER;
        }
        if n == 0 {
            b |= C_SPACE;
        }
        if n >= 9 && n <= 13 {
            b |= C_CONTROL;
        }
        t[n] = b;
        n += 1;
    }
    t
};

static HI: [u8; 16] = {
    let mut t = [0u8; 16];
    t[0] = C_CONTROL;
    t[2] = C_SPACE | C_DOLLAR;
    t[3] = C_DIGIT;
    t[4] = C_AO;
    t[5] = C_PZ | C_UNDER;
    t[6] = C_AO;
    t[7] = C_PZ;
    t
};

type Masks = [u64; MASKS];

/// One block's masks.
#[inline(always)]
unsafe fn masks<S: Simd8x64>(b: &[u8; 64]) -> Masks {
    unsafe {
        let v = S::load(b);
        let c = S::classify(v, &LO, &HI);
        let blank = S::to_bitmask(S::test(c, S::splat(C_BLANK)));
        let word = S::to_bitmask(S::test(c, S::splat(C_WORD))) | S::to_bitmask(S::ge(v, 0x80));
        let (star, slash, lf, bs) = (S::eq_mask(v, b'*'), S::eq_mask(v, b'/'), S::eq_mask(v, b'\n'), S::eq_mask(v, b'\\'));
        let (q, dq, nul) = (S::eq_mask(v, b'\''), S::eq_mask(v, b'"'), S::eq_mask(v, 0));
        [blank, word, star | slash, lf, q | bs, dq, nul]
    }
}

/// Stage 1 over `src`: each block's masks, the last block's padded.
#[inline(always)]
unsafe fn stage1<S: Simd8x64>(src: &[u8]) -> Vec<Masks> {
    let mut out = Vec::with_capacity(src.len().div_ceil(64));
    let mut at = 0;
    while let Some(b) = src.get(at..at + 64) {
        out.push(unsafe { masks::<S>(b.try_into().expect("64 bytes")) });
        at += 64;
    }
    if at < src.len() {
        let mut b = [0u8; 64];
        let n = src.len() - at;
        b[..n].copy_from_slice(&src[at..]);
        let mut m = unsafe { masks::<S>(&b) };
        let past = !0u64 << n;
        m[BLANK] &= !past;
        m[WORD] &= !past;
        m[NUL] &= !past;
        for k in [SLASH_STAR, LF, SQ, DQ] {
            m[k] |= past;
        }
        out.push(m);
    }
    out
}

rerail_simd8::dispatch! {
    fn stage1_best(src: &[u8]) -> Vec<Masks> = stage1, on stage1_on;
}

/// The text's masks (stage 1), for the parser's scans (stage 2).
pub struct Blocks<'s> {
    src: &'s [u8],
    m: Vec<Masks>,
}

impl<'s> Blocks<'s> {
    /// Stage 1 over `src`, on the best backend the CPU runs.
    pub fn of(src: &'s [u8]) -> Self {
        Blocks { src, m: stage1_best(src) }
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

    /// The end of a run of ASCII white space from `at`.
    #[inline]
    pub fn blank_end(&self, at: usize) -> usize {
        self.next(at, BLANK, false)
    }

    /// The first NUL in the text, if one is.
    #[inline]
    pub fn first_nul(&self) -> Option<usize> {
        let at = self.next(0, NUL, true);
        (at < self.src.len()).then_some(at)
    }

    /// The first ASCII white space byte from `at`, or the text's end.
    #[inline]
    pub fn blank_stop(&self, at: usize) -> usize {
        self.next(at, BLANK, true)
    }

    /// The end of a run of a word's bytes from `at` (`[A-Za-z0-9_$]`, and any byte from 0x80 on).
    #[inline]
    pub fn word_end(&self, at: usize) -> usize {
        self.next(at, WORD, false)
    }

    /// The end of the comment whose body starts at `at` (past its `*/`), nested comments closed
    /// inside it; `None` where it does not close.
    #[inline]
    pub fn comment_end(&self, mut at: usize) -> Option<usize> {
        let mut depth = 1u32;
        loop {
            let s = self.next(at, SLASH_STAR, true);
            if s + 1 >= self.src.len() {
                return None;
            }
            match (self.src[s], self.src[s + 1]) {
                (b'*', b'/') => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(s + 2);
                    }
                    at = s + 2;
                }
                (b'/', b'*') => {
                    depth += 1;
                    at = s + 2;
                }
                _ => at = s + 1,
            }
        }
    }

    /// The first `\n` from `at`, or the text's end.
    #[inline]
    pub fn line_end(&self, at: usize) -> usize {
        self.next(at, LF, true)
    }

    /// The first `'` or `\\` from `at`, or the text's end.
    #[inline]
    pub fn quote_stop(&self, at: usize) -> usize {
        self.next(at, SQ, true)
    }

    /// The first `"` from `at`, or the text's end.
    #[inline]
    pub fn dquote_stop(&self, at: usize) -> usize {
        self.next(at, DQ, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The masks one byte at a time.
    fn scalar(src: &[u8]) -> Vec<Masks> {
        let mut out = vec![[0u64; MASKS]; src.len().div_ceil(64)];
        for (b, o) in out.iter_mut().enumerate() {
            for i in 0..64 {
                let at = b * 64 + i;
                let Some(&c) = src.get(at) else {
                    for k in [SLASH_STAR, LF, SQ, DQ] {
                        o[k] |= 1 << i;
                    }
                    continue;
                };
                let bit = |x: bool| u64::from(x) << i;
                o[BLANK] |= bit(matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r'));
                o[WORD] |= bit(c.is_ascii_alphanumeric() || c == b'$' || c == b'_' || c >= 0x80);
                o[SLASH_STAR] |= bit(c == b'*' || c == b'/');
                o[LF] |= bit(c == b'\n');
                o[SQ] |= bit(matches!(c, b'\'' | b'\\'));
                o[DQ] |= bit(c == b'"');
                o[NUL] |= bit(c == 0);
            }
        }
        out
    }

    #[test]
    fn every_backend_gives_the_byte_tests_masks() {
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        let alphabet = b"aZ_9:-_$ \t\x0b\n\r\x0c'\"=/<>*\\#;\x7f^`";
        for len in [0usize, 1, 63, 64, 65, 200, 1000] {
            for _ in 0..20 {
                let src: Vec<u8> = (0..len)
                    .map(|_| {
                        x ^= x << 13;
                        x ^= x >> 7;
                        x ^= x << 17;
                        if x.is_multiple_of(3) { (x >> 16) as u8 } else { alphabet[(x >> 8) as usize % alphabet.len()] }
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
    }

    #[test]
    fn the_first_nul() {
        let mut t = vec![b'a'; 200];
        assert_eq!(Blocks::of(&t).first_nul(), None);
        t[130] = 0;
        t[199] = 0;
        assert_eq!(Blocks::of(&t).first_nul(), Some(130));
        assert_eq!(Blocks::of(&t[..100]).first_nul(), None);
    }

    #[test]
    fn scans_stop_where_they_should() {
        let b = Blocks::of("ab_c$9\u{e9} x".as_bytes());
        assert_eq!(b.word_end(0), 8);
        assert_eq!(Blocks::of(b"x  \n y").blank_end(1), 5);
        assert_eq!(Blocks::of(b"/* a * b */ c").comment_end(2), Some(11));
        assert_eq!(Blocks::of(b"/* a /* b */ c */ d").comment_end(2), Some(17));
        assert_eq!(Blocks::of(b"/* a *").comment_end(2), None);
        assert_eq!(Blocks::of(b"ab\"c'd").dquote_stop(0), 2);
        assert_eq!(Blocks::of(b"ab\\c'd").quote_stop(0), 2);
        assert_eq!(Blocks::of(b"-- x\ny").line_end(0), 4);
    }
}
