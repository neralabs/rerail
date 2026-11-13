//! Runs of bytes found 64 a step: a mask one bit a byte over a block, the run's end at the mask's
//! lowest bit (the micro-lexers' shape, `rr_syntax::micro`). A block past the text's end is padded
//! with a byte that ends every run (`\n`, and `\0`, which stops a word, a blank and a string).
//!
//! On aarch64 the compares are NEON's, packed into a mask as simdjson packs them; elsewhere eight
//! bytes a word. A test checks the two agree.

/// The byte classes a run is made of or stopped by, one bit each in `CLASS`.
const WORD: u8 = 1; // [A-Za-z0-9_]
const BLANK: u8 = 2; // ' ', '\t', '\f'
const EOL: u8 = 4; // '\n', '\r'
const HIGH: u8 = 8; // a byte from 0x80 (a character past ASCII)

static CLASS: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut c = 0;
    while c < 256 {
        let b = c as u8;
        t[c] = if b.is_ascii_alphanumeric() || b == b'_' {
            WORD
        } else if b == b' ' || b == b'\t' || b == 0x0c {
            BLANK
        } else if b == b'\n' || b == b'\r' {
            EOL
        } else if b >= 0x80 {
            HIGH
        } else {
            0
        };
        c += 1;
    }
    t
};

/// The 64 bytes from `at` (padded past the end).
#[inline(always)]
fn window(src: &[u8], at: usize) -> [u8; 64] {
    let mut b = [0u8; 64];
    let n = src.len().saturating_sub(at).min(64);
    b[..n].copy_from_slice(&src[at..at + n]);
    b
}

/// The first byte from `at` on whose class is not in `run`: a run's end (or `src.len()`).
#[inline]
fn run_end(src: &[u8], mut at: usize, run: u8) -> usize {
    while at < src.len() {
        let w = window(src, at);
        let stop = !masks::of(&w, run);
        if stop != 0 {
            return (at + stop.trailing_zeros() as usize).min(src.len());
        }
        at += 64;
    }
    src.len()
}

/// The end of a run of `[A-Za-z0-9_]` from `at`.
#[inline]
pub fn word_end(src: &[u8], at: usize) -> usize {
    // Most names are short: the first eight bytes a word before a block.
    run_end(src, at, WORD)
}

/// The end of a run of blanks (`' '`, `'\t'`, `'\f'`) from `at`.
#[inline]
pub fn blank_end(src: &[u8], at: usize) -> usize {
    match src.get(at) {
        Some(&c) if CLASS[usize::from(c)] & BLANK != 0 => run_end(src, at, BLANK),
        _ => at,
    }
}

/// The first `\n` or `\r` from `at` (or `src.len()`): a comment's end.
#[inline]
pub fn eol(src: &[u8], at: usize) -> usize {
    find(src, at, |w| masks::eq(w, b'\n') | masks::eq(w, b'\r'))
}

/// The first byte from `at` that may end a string's text: its quote, `\`, a line's end, or in a
/// format string `{` and `}`.
#[inline]
pub fn string_stop(src: &[u8], at: usize, quote: u8, format: bool) -> usize {
    if format {
        find(src, at, |w| masks::eq(w, quote) | masks::eq(w, b'\\') | masks::eq(w, b'\n') | masks::eq(w, b'{') | masks::eq(w, b'}'))
    } else {
        find(src, at, |w| masks::eq(w, quote) | masks::eq(w, b'\\') | masks::eq(w, b'\n'))
    }
}

/// The first byte from `at` that is in a format specifier's stops: `{`, `}`, a line's end.
#[inline]
pub fn spec_stop(src: &[u8], at: usize) -> usize {
    find(src, at, |w| masks::eq(w, b'{') | masks::eq(w, b'}') | masks::eq(w, b'\n'))
}

#[inline(always)]
fn find(src: &[u8], mut at: usize, mask: impl Fn(&[u8; 64]) -> u64) -> usize {
    while at < src.len() {
        let w = window(src, at);
        let m = mask(&w);
        if m != 0 {
            return (at + m.trailing_zeros() as usize).min(src.len());
        }
        at += 64;
    }
    src.len()
}

/// Whether `c` may continue a name (ASCII only: a byte from 0x80 is read as a character).
#[inline(always)]
pub fn is_word(c: u8) -> bool {
    CLASS[usize::from(c)] & WORD != 0
}

#[cfg(target_arch = "aarch64")]
mod masks {
    use std::arch::aarch64::{uint8x16_t, vandq_u8, vceqq_u8, vcgeq_u8, vcleq_u8, vdupq_n_u8, vgetq_lane_u64, vld1q_u8, vorrq_u8, vpaddq_u8, vreinterpretq_u64_u8};

    /// Bit `i`: byte `i` is `x`.
    #[inline(always)]
    pub(super) fn eq(w: &[u8; 64], x: u8) -> u64 {
        // SAFETY: `w` holds 64 bytes; NEON is part of every aarch64 target.
        unsafe {
            let s = vdupq_n_u8(x);
            let v = load(w);
            to_mask(vceqq_u8(v[0], s), vceqq_u8(v[1], s), vceqq_u8(v[2], s), vceqq_u8(v[3], s))
        }
    }

    /// Bit `i`: byte `i`'s class is in `run` (`super::WORD` or `super::BLANK`), by compares.
    #[inline(always)]
    pub(super) fn of(w: &[u8; 64], run: u8) -> u64 {
        // SAFETY: as `eq`.
        unsafe {
            let v = load(w);
            let range = |x: uint8x16_t, lo: u8, hi: u8| vandq_u8(vcgeq_u8(x, vdupq_n_u8(lo)), vcleq_u8(x, vdupq_n_u8(hi)));
            let one = |x: uint8x16_t| {
                if run == super::WORD {
                    let alpha = vorrq_u8(range(x, b'a', b'z'), range(x, b'A', b'Z'));
                    vorrq_u8(vorrq_u8(alpha, range(x, b'0', b'9')), vceqq_u8(x, vdupq_n_u8(b'_')))
                } else {
                    vorrq_u8(vorrq_u8(vceqq_u8(x, vdupq_n_u8(b' ')), vceqq_u8(x, vdupq_n_u8(b'\t'))), vceqq_u8(x, vdupq_n_u8(0x0c)))
                }
            };
            to_mask(one(v[0]), one(v[1]), one(v[2]), one(v[3]))
        }
    }

    #[inline(always)]
    unsafe fn load(w: &[u8; 64]) -> [uint8x16_t; 4] {
        // SAFETY: `w` holds 64 bytes.
        unsafe {
            let p = w.as_ptr();
            [vld1q_u8(p), vld1q_u8(p.add(16)), vld1q_u8(p.add(32)), vld1q_u8(p.add(48))]
        }
    }

    /// Four compare results as one mask, by simdjson's pairwise adds.
    #[inline(always)]
    unsafe fn to_mask(a: uint8x16_t, b: uint8x16_t, c: uint8x16_t, d: uint8x16_t) -> u64 {
        const BITS: [u8; 16] = [1, 2, 4, 8, 16, 32, 64, 128, 1, 2, 4, 8, 16, 32, 64, 128];
        // SAFETY: `BITS` holds 16 bytes; the rest are register operations.
        unsafe {
            let bits = vld1q_u8(BITS.as_ptr());
            let ab = vpaddq_u8(vandq_u8(a, bits), vandq_u8(b, bits));
            let cd = vpaddq_u8(vandq_u8(c, bits), vandq_u8(d, bits));
            let s = vpaddq_u8(ab, cd);
            let s = vpaddq_u8(s, s);
            vgetq_lane_u64(vreinterpretq_u64_u8(s), 0)
        }
    }
}

#[cfg(not(target_arch = "aarch64"))]
mod masks {
    pub(super) use super::portable::{eq, of};
}

#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
mod portable {
    const LO7: u64 = 0x7f7f_7f7f_7f7f_7f7f;
    const ONES: u64 = 0x0101_0101_0101_0101;

    #[inline(always)]
    fn words(w: &[u8; 64]) -> [u64; 8] {
        std::array::from_fn(|k| u64::from_le_bytes(w[8 * k..8 * k + 8].try_into().expect("8 bytes")))
    }

    /// Each byte's high bit: the byte is 0.
    #[inline(always)]
    fn zero(v: u64) -> u64 {
        !(((v & LO7) + LO7) | v) & !LO7
    }

    /// The 8 high bits of `hit` as a byte.
    #[inline(always)]
    fn gather(hit: u64) -> u64 {
        ((hit >> 7) & ONES).wrapping_mul(0x0102_0408_1020_4080) >> 56
    }

    pub(crate) fn eq(w: &[u8; 64], x: u8) -> u64 {
        words(w).iter().enumerate().fold(0, |m, (k, &v)| m | gather(zero(v ^ (ONES * u64::from(x)))) << (8 * k))
    }

    pub(crate) fn of(w: &[u8; 64], run: u8) -> u64 {
        let mut m = 0;
        for (i, &c) in w.iter().enumerate() {
            m |= u64::from(super::CLASS[usize::from(c)] & run != 0) << i;
        }
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_agree_with_the_portable_ones() {
        let mut w = [0u8; 64];
        for seed in 0..512u32 {
            for (i, b) in w.iter_mut().enumerate() {
                let x = (seed.wrapping_mul(2654435761).wrapping_add(i as u32 * 40503)) >> 7;
                *b = [b'a', b'Z', b'_', b'0', b' ', b'\t', b'\n', b'{', b'"', b'\\', 0xc3, b'.', 0x0c, b'9'][(x % 14) as usize];
            }
            for x in [b'\n', b'{', b'"', b'\\'] {
                assert_eq!(masks::eq(&w, x), portable::eq(&w, x));
            }
            for run in [WORD, BLANK] {
                assert_eq!(masks::of(&w, run), portable::of(&w, run));
            }
        }
    }

    #[test]
    fn runs_end_where_they_should() {
        let s = b"abc_9 \t x";
        assert_eq!(word_end(s, 0), 5);
        assert_eq!(blank_end(s, 5), 8);
        let long = [b'a'; 200];
        assert_eq!(word_end(&long, 3), 200);
        assert_eq!(eol(b"# x\r\n", 0), 3);
        assert_eq!(string_stop(b"ab{c'", 0, b'\'', true), 2);
        assert_eq!(string_stop(b"ab{c'", 0, b'\'', false), 4);
    }
}
