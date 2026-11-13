//! The text's byte classes, 64 bytes a step, as simdjson's two stages split the work:
//!
//! - **Stage 1** (`Blocks::of`) reads the whole text once, a 64-byte block a step, and keeps four
//!   masks a block, one bit a byte: a name's bytes, blanks, line ends, and the bytes that may end a
//!   string's text. The classes come from simdjson's nibble tables (`LO`, `HI`: a byte is in a
//!   class where both its nibbles' entries have the class's bit), the rest from compares. It is
//!   written once over `rerail_simd8`'s operations and compiled for each of simdjson's
//!   implementations (NEON, AVX2, SSE, the fallback); the best the CPU runs is chosen at run time.
//! - **Stage 2** is the lexer: a run's end, or the next byte of a class, is the lowest bit of a
//!   block's mask (`trailing_zeros`), the bits below the start cleared. Nothing in it is specific to a
//!   CPU.
//!
//! Past the text's end the runs (names, blanks) stop and the stops are all set, so a scan never
//! reads past the last block; its result is at most the text's length.

use rerail_simd8::Simd8x64;

/// A block's masks, by index.
const WORD: usize = 0; // [A-Za-z0-9_]
const BLANK: usize = 1; // ' ', '\t', '\f'
const EOL: usize = 2; // '\n', '\r'
const STOP: usize = 3; // what may end a string's text: '\'', '"', '`', '\\', '{', '}', '\n', '\r'

// The nibble tables' bits: each is the product of a set of high nibbles and a set of low ones.
const C_AO: u8 = 1; // high 4 or 6, low 1..=15: A-O, a-o
const C_PZ: u8 = 2; // high 5 or 7, low 0..=10: P-Z, p-z
const C_DIGIT: u8 = 4; // high 3, low 0..=9
const C_UNDERSCORE: u8 = 8; // high 5, low 15
const C_SPACE: u8 = 16; // high 2, low 0
const C_TAB_FF: u8 = 32; // high 0, low 9 or 12
const C_EOL: u8 = 64; // high 0, low 10 or 13
const C_QUOTE: u8 = 128; // high 2, low 2 or 7: '"', '\''
const C_WORD: u8 = C_AO | C_PZ | C_DIGIT | C_UNDERSCORE;
const C_BLANK: u8 = C_SPACE | C_TAB_FF;

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
        if n == 9 || n == 12 {
            b |= C_TAB_FF;
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
    t[0] = C_TAB_FF | C_EOL;
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

/// Whether `c` may continue a name (ASCII only: a byte from 0x80 is read as a character).
#[inline(always)]
pub fn is_word(c: u8) -> bool {
    class(c) & C_WORD != 0
}

/// One block's masks.
#[inline(always)]
unsafe fn masks<S: Simd8x64>(b: &[u8; 64]) -> [u64; 4] {
    unsafe {
        let v = S::load(b);
        let c = S::classify(v, &LO, &HI);
        // Each mask one test, packed once (`test` stays a vector).
        let word = S::to_bitmask(S::test(c, S::splat(C_WORD)));
        let blank = S::to_bitmask(S::test(c, S::splat(C_BLANK)));
        let eol = S::to_bitmask(S::test(c, S::splat(C_EOL)));
        let others = S::or(S::or(S::eq(v, S::splat(b'`')), S::eq(v, S::splat(b'\\'))), S::or(S::eq(v, S::splat(b'{')), S::eq(v, S::splat(b'}'))));
        let stop = S::to_bitmask(S::or(S::test(c, S::splat(C_EOL | C_QUOTE)), others));
        [word, blank, eol, stop]
    }
}

/// Stage 1 over `src`: each block's masks, the last block's padded.
#[inline(always)]
unsafe fn stage1<S: Simd8x64>(src: &[u8]) -> Vec<[u64; 4]> {
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
        let [word, blank, eol, stop] = unsafe { masks::<S>(&b) };
        let past = !0u64 << n;
        out.push([word & !past, blank & !past, eol | past, stop | past]);
    }
    out
}

rerail_simd8::dispatch! {
    fn stage1_best(src: &[u8]) -> Vec<[u64; 4]> = stage1, on stage1_on;
}

/// The text's masks (stage 1), for the lexer's scans (stage 2).
pub struct Blocks<'s> {
    src: &'s [u8],
    m: Vec<[u64; 4]>,
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

    /// The end of a run of `[A-Za-z0-9_]` from `at`.
    #[inline]
    pub fn word_end(&self, at: usize) -> usize {
        self.next(at, WORD, false)
    }

    /// The end of a run of blanks (`' '`, `'\t'`, `'\f'`) from `at`.
    #[inline]
    pub fn blank_end(&self, at: usize) -> usize {
        self.next(at, BLANK, false)
    }

    /// The first `\n` or `\r` from `at` (or the text's length): a comment's end.
    #[inline]
    pub fn eol(&self, at: usize) -> usize {
        self.next(at, EOL, true)
    }

    /// The first byte from `at` that may end a string's text: its quote, `\`, `\n`, or in a format
    /// string `{` and `}`. The stop mask holds every quote and brace and `\r`: the ones this string
    /// does not stop at are passed over a bit at a time.
    #[inline]
    pub fn string_stop(&self, at: usize, quote: u8, format: bool) -> usize {
        self.stop(at, |c| c == quote || c == b'\\' || c == b'\n' || (format && (c == b'{' || c == b'}')))
    }

    /// The first byte from `at` that is in a format specifier's stops: `{`, `}`, `\n`.
    #[inline]
    pub fn spec_stop(&self, at: usize) -> usize {
        self.stop(at, |c| c == b'{' || c == b'}' || c == b'\n')
    }

    #[inline(always)]
    fn stop(&self, mut at: usize, wanted: impl Fn(u8) -> bool) -> usize {
        loop {
            let s = self.next(at, STOP, true);
            if s >= self.src.len() || wanted(self.src[s]) {
                return s;
            }
            at = s + 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The masks one byte at a time.
    fn scalar(src: &[u8]) -> Vec<[u64; 4]> {
        let mut out = vec![[0u64; 4]; src.len().div_ceil(64)];
        for (b, o) in out.iter_mut().enumerate() {
            for i in 0..64 {
                let at = b * 64 + i;
                let Some(&c) = src.get(at) else {
                    o[EOL] |= 1 << i;
                    o[STOP] |= 1 << i;
                    continue;
                };
                let bit = |x: bool| u64::from(x) << i;
                o[WORD] |= bit(c.is_ascii_alphanumeric() || c == b'_');
                o[BLANK] |= bit(matches!(c, b' ' | b'\t' | 0x0c));
                o[EOL] |= bit(matches!(c, b'\n' | b'\r'));
                o[STOP] |= bit(matches!(c, b'\'' | b'"' | b'`' | b'\\' | b'{' | b'}' | b'\n' | b'\r'));
            }
        }
        out
    }

    #[test]
    fn every_backend_gives_the_byte_tests_masks() {
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        for len in [0usize, 1, 63, 64, 65, 200, 1000] {
            for _ in 0..20 {
                let src: Vec<u8> = (0..len)
                    .map(|_| {
                        x ^= x << 13;
                        x ^= x >> 7;
                        x ^= x << 17;
                        if x % 3 == 0 { (x >> 16) as u8 } else { b"aZ_9 \t\n\r\x0c'\"`\\{}#.(@[\x7f"[(x >> 8) as usize % 21] }
                    })
                    .collect();
                let want = scalar(&src);
                for be in rerail_simd8::Backend::ALL {
                    if let Some(got) = stage1_on(be, &src) {
                        assert_eq!(got, want, "{be:?}, {len} bytes");
                    }
                }
                for c in 0..=255u8 {
                    assert_eq!(is_word(c), c.is_ascii_alphanumeric() || c == b'_', "{c}");
                }
            }
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
        assert_eq!(Blocks::of(b"# x\r\n").eol(0), 3);
        let s = b"ab{c\"'";
        assert_eq!(Blocks::of(s).string_stop(0, b'\'', true), 2);
        assert_eq!(Blocks::of(s).string_stop(0, b'\'', false), 5);
        assert_eq!(Blocks::of(s).spec_stop(3), 6);
    }
}
