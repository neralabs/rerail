//! The text's byte classes, 64 bytes a step (stage 1, as in `rerail-css-lean`): a mask a class,
//! one bit a byte, from simdjson's nibble tables (`LO`, `HI`) and compares over `rerail_simd8`'s
//! operations, compiled for each backend. The scans (stage 2) take a run's end, or the next byte
//! of a class, as the lowest bit of a block's mask.
//!
//! Past the text's end the runs (blanks) stop and the stops are all set, so a scan never reads past
//! the last block; its result is at most the text's length.

use rerail_simd8::Simd8x64;

/// A block's masks, by index.
const BLANK: usize = 0; // ' ', '\t': the grammar's white space
const STOP: usize = 1; // where a word stops: white space, a line's end, ASCII punctuation
const EOL: usize = 2; // '\n', '\r'
const TICK_EOL: usize = 3; // '`', '\n', '\r'
const TAB: usize = 4; // '\t'
const CONT: usize = 5; // a UTF-8 continuation byte (0x80 to 0xbf)
const MASKS: usize = 6;

// The nibble tables' bits for STOP: each is the product of a set of high nibbles and a set of low
// ones.
const C_2X: u8 = 1; // high 2, any low: ' ' and !"#$%&'()*+,-./
const C_3X: u8 = 2; // high 3, low 10..=15: :;<=>?
const C_X0: u8 = 4; // high 4 or 6, low 0: @ `
const C_5X: u8 = 8; // high 5, low 11..=15: [\]^_
const C_7X: u8 = 16; // high 7, low 11..=14: {|}~
const C_CTL: u8 = 32; // high 0, low 9, 10, 13: '\t', '\n', '\r'
const C_STOP: u8 = C_2X | C_3X | C_X0 | C_5X | C_7X | C_CTL;

static LO: [u8; 16] = {
    let mut t = [0u8; 16];
    let mut n = 0;
    while n < 16 {
        let mut b = C_2X;
        if n >= 10 {
            b |= C_3X;
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
    t[3] = C_3X;
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
        let stop = S::to_bitmask(S::test(S::classify(v, &LO, &HI), S::splat(C_STOP)));
        let (sp, tab) = (S::eq_mask(v, b' '), S::eq_mask(v, b'\t'));
        let eol = S::eq_mask(v, b'\n') | S::eq_mask(v, b'\r');
        let cont = S::to_bitmask(S::ge(v, 0x80)) & !S::to_bitmask(S::ge(v, 0xc0));
        [sp | tab, stop, eol, eol | S::eq_mask(v, b'`'), tab, cont]
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
        for k in [BLANK, TAB, CONT] {
            m[k] &= !past;
        }
        for k in [STOP, EOL, TICK_EOL] {
            m[k] |= past;
        }
        out.push(m);
    }
    out
}

rerail_simd8::dispatch! {
    fn stage1_best(src: &[u8]) -> Vec<Masks> = stage1, on stage1_on;
}

/// The text's masks (stage 1), for the scans (stage 2).
pub struct Blocks {
    m: rerail_simd8::blocks::Masks<MASKS>,
}

impl Blocks {
    /// Stage 1 over `src`, on the best backend the CPU runs.
    pub fn of(src: &[u8]) -> Self {
        Blocks { m: rerail_simd8::blocks::Masks::new(src.len(), stage1_best(src)) }
    }

    /// The first byte from `at` on whose bit in mask `k` is `set`.
    #[inline(always)]
    fn next(&self, at: usize, k: usize, set: bool) -> usize {
        self.m.next(at, k, set)
    }

    /// Mask `k`'s bits in `from..to`, block by block: `f(block, bits)`.
    #[inline(always)]
    fn each(&self, from: usize, to: usize, k: usize, mut f: impl FnMut(usize, u64)) {
        if from >= to {
            return;
        }
        let (first, last) = (from / 64, (to - 1) / 64);
        for b in first..=last {
            let mut m = self.m.mask(b, k);
            if b == first {
                m &= !0u64 << (from % 64);
            }
            if b == last && to % 64 != 0 {
                m &= !(!0u64 << (to % 64));
            }
            f(b, m);
        }
    }

    /// The end of a run of white space (`' '`, `'\t'`) from `at`.
    #[inline]
    pub fn blank_end(&self, at: usize) -> usize {
        self.next(at, BLANK, false)
    }

    /// Where a word from `at` stops: white space, a line's end, ASCII punctuation, or the text's
    /// end.
    #[inline]
    pub fn word_end(&self, at: usize) -> usize {
        self.next(at, STOP, true)
    }

    /// The first `'\n'` or `'\r'` from `at`, or the text's end.
    #[inline]
    pub fn line_end(&self, at: usize) -> usize {
        self.next(at, EOL, true)
    }

    /// The first `` ` ``, `'\n'` or `'\r'` from `at`, or the text's end.
    #[inline]
    pub fn tick_or_line_end(&self, at: usize) -> usize {
        self.next(at, TICK_EOL, true)
    }

    /// The scanner's column (mod 4) after advancing over `from..to` from column `column`, a
    /// character at a time (`advance`: a tab to its next stop, any other character one on).
    pub fn column_after(&self, from: usize, to: usize, column: u8) -> u8 {
        let mut last_tab = None;
        self.each(from, to, TAB, |b, m| {
            if m != 0 {
                last_tab = Some(b * 64 + 63 - m.leading_zeros() as usize);
            }
        });
        let (start, base) = match last_tab {
            Some(t) => (t + 1, 0),
            None => (from, u32::from(column)),
        };
        let mut cont = 0u32;
        self.each(start, to, CONT, |_, m| cont += m.count_ones());
        ((base + (to - start) as u32 - cont) % 4) as u8
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
                    for k in [STOP, EOL, TICK_EOL] {
                        o[k] |= 1 << i;
                    }
                    continue;
                };
                let bit = |x: bool| u64::from(x) << i;
                o[BLANK] |= bit(matches!(c, b' ' | b'\t'));
                o[STOP] |= bit(matches!(c, b' ' | b'\t' | b'\n' | b'\r') || c.is_ascii_punctuation());
                o[EOL] |= bit(matches!(c, b'\n' | b'\r'));
                o[TICK_EOL] |= bit(matches!(c, b'`' | b'\n' | b'\r'));
                o[TAB] |= bit(c == b'\t');
                o[CONT] |= bit((0x80..0xc0).contains(&c));
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
                        (x >> 16) as u8
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
    fn scans_stop_where_they_should() {
        let b = Blocks::of("ab\u{e9}c-d e".as_bytes());
        assert_eq!(b.word_end(0), 5);
        assert_eq!(Blocks::of(b"x \t\n y").blank_end(1), 3);
        assert_eq!(Blocks::of(b"ab`c\nd").tick_or_line_end(0), 2);
        assert_eq!(Blocks::of(b"abc\r\nd").line_end(0), 3);
    }

    #[test]
    fn the_column_is_advances() {
        // One character at a time, as the scanner advances.
        let advance = |src: &[u8], from: usize, to: usize, mut col: u8| {
            let s = std::str::from_utf8(&src[from..to]).unwrap();
            for c in s.chars() {
                col = if c == '\t' { 0 } else { (col + 1) % 4 };
            }
            col
        };
        for text in ["abc", "a\tbc", "\u{e9}\u{4e2d}x\ty\u{1f600}", "\t", "x".repeat(130).as_str(), &format!("{}\t{}", "\u{e9}".repeat(40), "z".repeat(70))] {
            let src = text.as_bytes();
            let b = Blocks::of(src);
            for col in 0..4 {
                assert_eq!(b.column_after(0, src.len(), col), advance(src, 0, src.len(), col), "{text:?} from {col}");
            }
        }
    }
}
