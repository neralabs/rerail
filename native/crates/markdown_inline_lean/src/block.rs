//! The text's byte classes, 64 bytes a step (stage 1, as in `rerail-markdown-lean`): a mask a
//! class, one bit a byte, from simdjson's nibble tables (`LO`, `HI`) and compares over
//! `rerail_simd8`'s operations, compiled for each backend. The scans (stage 2) take a run's end,
//! or the next byte of a class, as the lowest bit of a block's mask.
//!
//! Past the text's end the runs stop and the stops are all set, so a scan never reads past the
//! last block; its result is at most the text's length.

use rerail_simd8::Simd8x64;

/// A block's masks, by index.
const BLANK: usize = 0; // ' ', '\t': the grammar's white space
const WORD_STOP: usize = 1; // where a word's characters stop: white space, a line's end, ASCII punctuation, a digit
const DIGITS: usize = 2; // a digit or '_': `_digits`' characters after its first
const TICK: usize = 3; // '`'
const DOLLAR: usize = 4; // '$'
const NUL: usize = 5; // a NUL, which the parser refuses
const MASKS: usize = 6;

// The nibble tables' bits for the stops: each is the product of a set of high nibbles and a set of
// low ones.
const C_2X: u8 = 1; // high 2, any low: ' ' and !"#$%&'()*+,-./
const C_3X: u8 = 2; // high 3, low 10..=15: :;<=>?
const C_X0: u8 = 4; // high 4 or 6, low 0: @ `
const C_5X: u8 = 8; // high 5, low 11..=15: [\]^_
const C_7X: u8 = 16; // high 7, low 11..=14: {|}~
const C_CTL: u8 = 32; // high 0, low 9, 10, 13: '\t', '\n', '\r'
const C_DIGIT: u8 = 64; // high 3, low 0..=9
const C_STOP: u8 = C_2X | C_3X | C_X0 | C_5X | C_7X | C_CTL | C_DIGIT;

static LO: [u8; 16] = {
    let mut t = [0u8; 16];
    let mut n = 0;
    while n < 16 {
        let mut b = C_2X;
        if n >= 10 {
            b |= C_3X;
        } else {
            b |= C_DIGIT;
        }
        if n == 0 {
            b |= C_X0;
        }
        if n >= 11 {
            b |= C_5X;
        }
        if n >= 11 && n <= 14 {
            b |= C_7X;
        }
        if n == 9 || n == 10 || n == 13 {
            b |= C_CTL;
        }
        t[n] = b;
        n += 1;
    }
    t
};

static HI: [u8; 16] = {
    let mut t = [0u8; 16];
    t[0] = C_CTL;
    t[2] = C_2X;
    t[3] = C_3X | C_DIGIT;
    t[4] = C_X0;
    t[5] = C_5X;
    t[6] = C_X0;
    t[7] = C_7X;
    t
};

type Masks = [u64; MASKS];

/// One block's masks.
#[inline(always)]
unsafe fn masks<S: Simd8x64>(b: &[u8; 64]) -> Masks {
    unsafe {
        let v = S::load(b);
        let c = S::classify(v, &LO, &HI);
        let stop = S::to_bitmask(S::test(c, S::splat(C_STOP)));
        let digit = S::to_bitmask(S::test(c, S::splat(C_DIGIT)));
        let blank = S::eq_mask(v, b' ') | S::eq_mask(v, b'\t');
        [blank, stop, digit | S::eq_mask(v, b'_'), S::eq_mask(v, b'`'), S::eq_mask(v, b'$'), S::eq_mask(v, 0)]
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
        for k in [BLANK, DIGITS, TICK, DOLLAR, NUL] {
            m[k] &= !past;
        }
        m[WORD_STOP] |= past;
        out.push(m);
    }
    out
}

rerail_simd8::dispatch! {
    fn stage1_best(src: &[u8]) -> Vec<Masks> = stage1, on stage1_on;
}

/// The text's masks (stage 1), for the scans (stage 2).
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

    /// The first NUL in the text, if one is.
    #[inline]
    pub fn first_nul(&self) -> Option<usize> {
        let at = self.next(0, NUL, true);
        (at < self.src.len()).then_some(at)
    }

    /// The end of a run of white space (`' '`, `'\t'`) from `at`.
    #[inline]
    pub fn blank_end(&self, at: usize) -> usize {
        self.next(at, BLANK, false)
    }

    /// Where a word's characters from `at` stop: white space, a line's end, ASCII punctuation, a
    /// digit, or the text's end.
    #[inline]
    pub fn word_stop(&self, at: usize) -> usize {
        self.next(at, WORD_STOP, true)
    }

    /// The end of a run of digits and `_` from `at`.
    #[inline]
    pub fn digits_end(&self, at: usize) -> usize {
        self.next(at, DIGITS, false)
    }

    /// The first `` ` `` from `at`, or the text's end.
    #[inline]
    pub fn next_tick(&self, at: usize) -> usize {
        self.next(at, TICK, true)
    }

    /// The end of a run of `` ` `` from `at`.
    #[inline]
    pub fn tick_end(&self, at: usize) -> usize {
        self.next(at, TICK, false)
    }

    /// The first `$` from `at`, or the text's end.
    #[inline]
    pub fn next_dollar(&self, at: usize) -> usize {
        self.next(at, DOLLAR, true)
    }

    /// The end of a run of `$` from `at`.
    #[inline]
    pub fn dollar_end(&self, at: usize) -> usize {
        self.next(at, DOLLAR, false)
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
                let Some(&c) = src.get(b * 64 + i) else {
                    o[WORD_STOP] |= 1 << i;
                    continue;
                };
                let bit = |x: bool| u64::from(x) << i;
                o[BLANK] |= bit(c == b' ' || c == b'\t');
                o[WORD_STOP] |= bit(c.is_ascii_punctuation() || matches!(c, b' ' | b'\t' | b'\n' | b'\r') || c.is_ascii_digit());
                o[DIGITS] |= bit(c.is_ascii_digit() || c == b'_');
                o[TICK] |= bit(c == b'`');
                o[DOLLAR] |= bit(c == b'$');
                o[NUL] |= bit(c == 0);
            }
        }
        out
    }

    #[test]
    fn every_backend_gives_the_byte_tests_masks() {
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        let alphabet = b"aZ_9:-_$ \t\n\r'\"=/<>*\\#;\x7f^`[]{}~0";
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
        let b = Blocks::of(b"ab1_2 `` $$x");
        assert_eq!(b.word_stop(0), 2);
        assert_eq!(b.digits_end(2), 5);
        assert_eq!(b.blank_end(5), 6);
        assert_eq!((b.next_tick(0), b.tick_end(6)), (6, 8));
        assert_eq!((b.next_dollar(0), b.dollar_end(9)), (9, 11));
    }
}
