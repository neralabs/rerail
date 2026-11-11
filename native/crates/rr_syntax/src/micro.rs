//! Micro-lexers: the end of one construct, found 64 bytes a step from where it starts
//! (docs/parser-generators/31-micro-lexers.md).
//!
//! Each is Bitlex's block shape, run from a token's start rather than over the whole file:
//!
//! - **A block is 64 bytes, a mask one bit a byte**, as Bitlex's tier 1 and simdjson make them.
//!   A block past the text's end is padded with zeros, which no micro-lexer stops at.
//! - **The masks are Bitlex's operations**: `eq` (a byte compared, all 64 at once), `pair` (a
//!   byte followed by another) and `escaped` (simdjson's branch-free escape finder, Bitlex's
//!   own formula).
//! - **No branch inside a block.** A micro-lexer makes its stop mask; the construct ends at the
//!   mask's lowest bit (a trailing-zero count). The one branch a block is "no stop here yet".
//! - **What else the construct needs is found in the same step**: a `\u` escape in a string
//!   (escaped `u`s under the stop).
//!
//! On aarch64 the masks come from NEON compares, packed as simdjson packs them. Elsewhere,
//! 8 bytes a word (`portable`); a test checks the two agree.

#[cfg(target_arch = "aarch64")]
use neon::Block;
#[cfg(not(target_arch = "aarch64"))]
use portable::Block;

#[cfg(target_arch = "aarch64")]
mod neon {
    use std::arch::aarch64::{
        uint8x16_t, vandq_u8, vceqq_u8, vcgeq_u8, vdupq_n_u8, vextq_u8, vgetq_lane_u64, vld1q_u8, vorrq_u8, vpaddq_u8, vqtbl1q_u8,
        vreinterpretq_u64_u8, vshrq_n_u8, vtstq_u8,
    };
    use super::{FIRST_HI, FIRST_LO, HI, LO, RARE, SECOND_HI, SECOND_LO, SPACE, WORD};

    /// The lexer's four masks over the 64 bytes `p[..64]`; `p[64..]` are the bytes after, for
    /// the pairs' second bytes (`super::window`).
    #[inline(always)]
    pub(super) fn lexer(p: &[u8; 80]) -> [u64; 4] {
        // SAFETY: `p` holds 80 bytes, the tables 16 each; the rest are register operations.
        unsafe {
            let at = |k: usize| vld1q_u8(p.as_ptr().add(16 * k));
            let v = [at(0), at(1), at(2), at(3), at(4)];
            let t = |x: &[u8; 16]| vld1q_u8(x.as_ptr());
            let (lo, hi, flo, fhi, slo, shi) = (t(&LO), t(&HI), t(&FIRST_LO), t(&FIRST_HI), t(&SECOND_LO), t(&SECOND_HI));
            let (low, lead) = (vdupq_n_u8(0x0f), vdupq_n_u8(0xc0));
            let (space, word, rare) = (vdupq_n_u8(SPACE), vdupq_n_u8(WORD), vdupq_n_u8(RARE));
            let mut out = [[vdupq_n_u8(0); 4]; 4];
            for k in 0..4 {
                let x = v[k];
                let (xl, xh) = (vandq_u8(x, low), vshrq_n_u8::<4>(x));
                let c = vandq_u8(vqtbl1q_u8(lo, xl), vqtbl1q_u8(hi, xh));
                out[0][k] = vtstq_u8(c, space);
                out[1][k] = vtstq_u8(c, word);
                out[2][k] = vorrq_u8(vtstq_u8(c, rare), vcgeq_u8(x, lead));
                // A pair: the first byte's table names the second bytes it takes, the next byte's
                // which it is.
                let f = vandq_u8(vqtbl1q_u8(flo, xl), vqtbl1q_u8(fhi, xh));
                let d = vextq_u8::<1>(x, v[k + 1]);
                let s = vandq_u8(vqtbl1q_u8(slo, vandq_u8(d, low)), vqtbl1q_u8(shi, vshrq_n_u8::<4>(d)));
                out[3][k] = vtstq_u8(f, s);
            }
            out.map(|[a, b, c, d]| to_mask(a, b, c, d))
        }
    }

    /// 64 bytes in four vectors.
    pub(super) struct Block([uint8x16_t; 4]);

    impl Block {
        #[inline(always)]
        pub(super) fn load(b: &[u8; 64]) -> Self {
            // SAFETY: `b` holds 64 bytes; NEON is part of every aarch64 target.
            unsafe {
                let p = b.as_ptr();
                Block([vld1q_u8(p), vld1q_u8(p.add(16)), vld1q_u8(p.add(32)), vld1q_u8(p.add(48))])
            }
        }

        /// Bit `i`: byte `i` is `x`.
        #[inline(always)]
        pub(super) fn eq(&self, x: u8) -> u64 {
            // SAFETY: register operations only.
            unsafe {
                let s = vdupq_n_u8(x);
                let [a, b, c, d] = self.0;
                to_mask(vceqq_u8(a, s), vceqq_u8(b, s), vceqq_u8(c, s), vceqq_u8(d, s))
            }
        }

    }

    /// Four compare results as one mask, by simdjson's pairwise adds: each byte keeps its bit of
    /// its group of 8, and the adds gather the groups.
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

#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
mod portable {
    /// 64 bytes in eight words.
    pub(super) struct Block([u64; 8]);

    impl Block {
        #[inline(always)]
        pub(super) fn load(b: &[u8; 64]) -> Self {
            Block(std::array::from_fn(|k| u64::from_le_bytes(b[8 * k..8 * k + 8].try_into().expect("8 bytes"))))
        }

        #[inline(always)]
        pub(super) fn eq(&self, x: u8) -> u64 {
            const LO7: u64 = 0x7f7f_7f7f_7f7f_7f7f;
            const ONES: u64 = 0x0101_0101_0101_0101;
            let mut m = 0;
            for (k, &w) in self.0.iter().enumerate() {
                let v = w ^ (ONES * u64::from(x));
                // Each equal byte's low bit, then the 8 gathered into one byte.
                let hit = (!(((v & LO7) + LO7) | v) >> 7) & ONES;
                m |= (hit.wrapping_mul(0x0102_0408_1020_4080) >> 56) << (8 * k);
            }
            m
        }

    }

    /// `neon::lexer`, a byte at a time.
    pub(super) fn lexer(p: &[u8; 80]) -> [u64; 4] {
        use super::{FIRST_HI, FIRST_LO, HI, LO, RARE, SECOND_HI, SECOND_LO, SPACE, WORD};
        let class = |lo: &[u8; 16], hi: &[u8; 16], c: u8| lo[usize::from(c & 15)] & hi[usize::from(c >> 4)];
        let mut out = [0u64; 4];
        for i in 0..64 {
            let (x, d) = (p[i], p[i + 1]);
            let c = class(&LO, &HI, x);
            let bits = [c & SPACE != 0, c & WORD != 0, c & RARE != 0 || x >= 0xc0, class(&FIRST_LO, &FIRST_HI, x) & class(&SECOND_LO, &SECOND_HI, d) != 0];
            for (m, bit) in out.iter_mut().zip(bits) {
                *m |= u64::from(bit) << i;
            }
        }
        out
    }

}

/// The block at `i`: 64 bytes, zeros past the end.
#[inline(always)]
fn block(b: &[u8], i: usize) -> Block {
    match b.get(i..i + 64) {
        Some(s) => Block::load(s.try_into().expect("64 bytes")),
        None => {
            let mut pad = [0u8; 64];
            pad[..b.len() - i].copy_from_slice(&b[i..]);
            Block::load(&pad)
        }
    }
}

// The lexer's classes, as nibble tables: each bit a set of high nibbles times a set of low
// ones, a class the bits it is the union of.
const C_SPACE: u8 = 1; // ' '
const C_CONTROL: u8 = 2; // \t \n \x0c \r
const C_DIGIT: u8 = 4; // 0-9
const C_LETTER_1: u8 = 8; // A-O, a-o (less @, `)
const C_LETTER_2: u8 = 16; // P-Z, p-z
const C_UNDERSCORE: u8 = 32;
const C_QUOTE_HASH_SLASH: u8 = 64; // " # /
const C_BACKTICK: u8 = 128;
/// Whitespace, as `u8::is_ascii_whitespace`.
const SPACE: u8 = C_SPACE | C_CONTROL;
/// A word's bytes: letters, digits and `_`.
const WORD: u8 = C_DIGIT | C_LETTER_1 | C_LETTER_2 | C_UNDERSCORE;
/// What a rare token starts with (a non-ASCII character's first byte aside, by a compare): a
/// string, a backtick literal, `#`, a comment or `/`, a number.
const RARE: u8 = C_QUOTE_HASH_SLASH | C_BACKTICK | C_DIGIT;
const LO: [u8; 16] = {
    let mut t = [0u8; 16];
    let mut l = 0;
    while l < 16 {
        let mut c = C_LETTER_1 * (l >= 1) as u8 | C_LETTER_2 * (l <= 10) as u8 | C_DIGIT * (l <= 9) as u8;
        c |= C_SPACE * (l == 0) as u8 | C_CONTROL * matches!(l, 9 | 10 | 12 | 13) as u8 | C_UNDERSCORE * (l == 15) as u8;
        c |= C_QUOTE_HASH_SLASH * matches!(l, 2 | 3 | 15) as u8 | C_BACKTICK * (l == 0) as u8;
        t[l] = c;
        l += 1;
    }
    t
};
const HI: [u8; 16] = [
    C_CONTROL,
    0,
    C_SPACE | C_QUOTE_HASH_SLASH,
    C_DIGIT,
    C_LETTER_1,
    C_LETTER_2 | C_UNDERSCORE,
    C_LETTER_1 | C_BACKTICK,
    C_LETTER_2,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
];

// The two-byte punctuation (`:-`, `=>`, `!=`, `<=`, `>=`, `<:`, `->`) as two nibble-table pairs:
// a first byte's bits are the second bytes it takes, a second byte's the one it is; a pair is
// any bit of both. Each bit a product of nibble sets: `:` (3, a) takes `-`; `=` and `-`
// (2|3, d) take `>`; `!` (2, 1), and `<` and `>` (3, c|e), take `=`; `<` (3, c) takes `:`.
const P_MINUS: u8 = 1;
const P_GT: u8 = 2;
const P_EQ_1: u8 = 4;
const P_EQ_2: u8 = 8;
const P_COLON: u8 = 16;
const FIRST_LO: [u8; 16] = {
    let mut t = [0u8; 16];
    t[0xa] = P_MINUS;
    t[0xd] = P_GT;
    t[0x1] = P_EQ_1;
    t[0xc] = P_EQ_2 | P_COLON;
    t[0xe] = P_EQ_2;
    t
};
const FIRST_HI: [u8; 16] = {
    let mut t = [0u8; 16];
    t[2] = P_GT | P_EQ_1;
    t[3] = P_MINUS | P_GT | P_EQ_2 | P_COLON;
    t
};
// `-` (2, d), `>` (3, e), `=` (3, d), `:` (3, a).
const SECOND_LO: [u8; 16] = {
    let mut t = [0u8; 16];
    t[0xd] = P_MINUS | P_EQ_1 | P_EQ_2;
    t[0xe] = P_GT;
    t[0xa] = P_COLON;
    t
};
const SECOND_HI: [u8; 16] = {
    let mut t = [0u8; 16];
    t[2] = P_MINUS;
    t[3] = P_GT | P_EQ_1 | P_EQ_2 | P_COLON;
    t
};

/// A lexer's window: the 64 bytes from `base`, as masks a bit a byte.
pub(crate) struct Window {
    pub base: usize,
    pub space: u64,
    pub word: u64,
    /// What a rare token starts with: `"`, `` ` ``, `#`, `/`, a digit, a non-ASCII character's
    /// first byte.
    pub rare: u64,
    /// A two-byte punctuation's first byte: the byte after it is its second.
    pub pair: u64,
    /// The bytes: the 64, and 16 after (zeros past the text), so a token's first bytes are
    /// read without a bounds check.
    pub bytes: [u8; 80],
}

impl Window {
    #[inline(always)]
    pub(crate) fn at(b: &[u8], base: usize) -> Self {
        let bytes: [u8; 80] = match b.get(base..base + 80) {
            Some(s) => s.try_into().expect("80 bytes"),
            None => {
                let mut pad = [0u8; 80];
                let s = &b[base..];
                pad[..s.len()].copy_from_slice(s);
                pad
            }
        };
        #[cfg(target_arch = "aarch64")]
        let [space, word, rare, pair] = neon::lexer(&bytes);
        #[cfg(not(target_arch = "aarch64"))]
        let [space, word, rare, pair] = portable::lexer(&bytes);
        Window { base, space, word, rare, pair, bytes }
    }
}

/// Where each newline of `b` is: a block's `eq` mask, its bits taken one at a time.
pub(crate) fn newline_positions(b: &[u8]) -> Vec<u32> {
    let mut out = Vec::with_capacity(b.len() / 32);
    let mut i = 0;
    while i < b.len() {
        let mut m = block(b, i).eq(b'\n');
        while m != 0 {
            out.push((i + m.trailing_zeros() as usize) as u32);
            m &= m - 1;
        }
        i += 64;
    }
    out
}

/// The bits `from..to` of a mask (`from` < 64, `to` ≤ 64).
#[inline(always)]
pub(crate) fn bits(from: u32, to: u32) -> u64 {
    let upto = if to >= 64 { !0 } else { (1u64 << to) - 1 };
    upto & !((1u64 << from) - 1)
}

/// The run of `mask`'s bits from bit `from` (< 64): its length, to 64 at most.
#[inline(always)]
pub(crate) fn run(mask: u64, from: u32) -> u32 {
    (!(mask >> from)).trailing_zeros().min(64 - from)
}

/// Bitlex's `escaped`: the bytes a run of backslashes escapes (simdjson's
/// `find_escaped_branchless`), `cin` the block's first byte escaped; the carry out.
#[inline(always)]
fn escaped(run: u64, cin: u64) -> (u64, u64) {
    const EVEN: u64 = 0x5555_5555_5555_5555;
    let b = run & !cin;
    let follows = (b << 1) | cin;
    let odd_starts = b & !EVEN & !follows;
    let (sequences, overflow) = odd_starts.overflowing_add(b);
    ((EVEN ^ (sequences << 1)) & follows, u64::from(overflow))
}

/// The bits below a mask's lowest bit (all of them when it is empty).
#[inline(always)]
fn below(stop: u64) -> u64 {
    stop.wrapping_sub(1) & !stop
}

/// From `from` on, block by block, to the first bit of `stop`'s mask (`None`: the text ends
/// first).
#[inline(always)]
fn scan(b: &[u8], from: usize, mut stop: impl FnMut(&Block, usize) -> u64) -> Option<usize> {
    let mut i = from;
    while i < b.len() {
        let s = stop(&block(b, i), i);
        if s != 0 {
            return Some(i + s.trailing_zeros() as usize);
        }
        i += 64;
    }
    None
}

/// The first `x` from `from` on: a line comment's end, a backtick literal's.
pub(crate) fn find(b: &[u8], from: usize, x: u8) -> Option<usize> {
    scan(b, from, |blk, _| blk.eq(x))
}

/// The first `x` followed by `y` from `from` on (Bitlex's `pair`: a block comment's `*/`, the
/// fence of a two-backtick literal).
pub(crate) fn find_pair(b: &[u8], from: usize, x: u8, y: u8) -> Option<usize> {
    scan(b, from, |blk, i| {
        // The next block's first byte, for this block's last.
        let next = u64::from(b.get(i + 64) == Some(&y));
        blk.eq(x) & ((blk.eq(y) >> 1) | (next << 63))
    })
}

/// A string's body from `from` (after its opening quote): its closing quote, the first not
/// escaped; and whether a `\u` escape comes before it.
pub(crate) fn string_end(b: &[u8], from: usize) -> (Option<usize>, bool) {
    let (mut carry, mut u) = (0, 0);
    let end = scan(b, from, |blk, _| {
        let (esc, out) = escaped(blk.eq(b'\\'), carry);
        carry = out;
        let quote = blk.eq(b'"') & !esc;
        u |= esc & blk.eq(b'u') & below(quote);
        quote
    });
    (end, u != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small generator, so the tests need no crate.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        /// Text of the bytes a micro-lexer stops at, and others.
        fn text(&mut self, n: usize) -> Vec<u8> {
            const ALPHABET: &[u8] = b"\"\\\\u`*/\nab x";
            (0..n).map(|_| ALPHABET[(self.next() % ALPHABET.len() as u64) as usize]).collect()
        }
    }

    #[test]
    fn masks_are_the_portable_ones() {
        let mut r = Rng(0x9e37_79b9_7f4a_7c15);
        for _ in 0..2000 {
            let t: [u8; 64] = std::array::from_fn(|_| r.next() as u8 % 8);
            let x = r.next() as u8 % 8;
            let want = (0..64).fold(0u64, |m, i| m | (u64::from(t[i] == x) << i));
            assert_eq!(Block::load(&t).eq(x), want);
            assert_eq!(portable::Block::load(&t).eq(x), want);
        }
    }

    #[test]
    fn lexer_masks_are_the_byte_tests() {
        let mut r = Rng(0x1234_5678_9abc_def1);
        let pairs: [&[u8]; 7] = [b":-", b"=>", b"!=", b"<=", b">=", b"<:", b"->"];
        for round in 0..3000 {
            // Every byte value, and texts of the pairs' bytes.
            let t: Vec<u8> = if round < 4 {
                (0..=255u8).cycle().skip(round * 64).take(150).collect()
            } else {
                (0..150).map(|_| b":-=>!<a\"#/`0 \xc3\x80"[(r.next() % 14) as usize]).collect()
            };
            let base = (r.next() % 100) as usize;
            let w = Window::at(&t, base);
            let mut pad = [0u8; 80];
            let s = &t[base..(base + 80).min(t.len())];
            pad[..s.len()].copy_from_slice(s);
            assert_eq!([w.space, w.word, w.rare, w.pair], portable::lexer(&pad));
            for i in 0..64 {
                let c = pad[i];
                let rare = matches!(c, b'"' | b'`' | b'#' | b'/') || c.is_ascii_digit() || c >= 0xc0;
                assert_eq!(w.rare >> i & 1 == 1, rare, "{c:#x}");
                assert_eq!(w.pair >> i & 1 == 1, pairs.contains(&&pad[i..i + 2]), "{:?}", &pad[i..i + 2]);
                assert_eq!(w.space >> i & 1 == 1, c.is_ascii_whitespace(), "{c:#x}");
                assert_eq!(w.word >> i & 1 == 1, c.is_ascii_alphanumeric() || c == b'_', "{c:#x}");
            }
        }
    }

    #[test]
    fn micro_lexers_are_the_scalar_scans() {
        let mut r = Rng(0x2545_f491_4f6c_dd1d);
        for _ in 0..20_000 {
            let n = (r.next() % 200) as usize;
            let b = r.text(n);
            let from = if n == 0 { 0 } else { (r.next() % n as u64) as usize };
            // `find`
            let want = b[from..].iter().position(|&c| c == b'`').map(|e| from + e);
            assert_eq!(find(&b, from, b'`'), want);
            // `find_pair`, as `str::find("*/")`
            let want = b[from..].windows(2).position(|w| w == b"*/").map(|e| from + e);
            assert_eq!(find_pair(&b, from, b'*', b'/'), want, "{:?} from {from}", String::from_utf8_lossy(&b));
            // `string_end`, as the engine's lexer walks a string
            let (mut end, mut u) = (from, false);
            while end < n && b[end] != b'"' {
                if b[end] == b'\\' {
                    u |= b.get(end + 1) == Some(&b'u');
                    end += 2;
                } else {
                    end += 1;
                }
            }
            let (at, got_u) = string_end(&b, from);
            if end < n {
                assert_eq!(at, Some(end), "{:?} from {from}", String::from_utf8_lossy(&b));
                assert_eq!(got_u, u);
            } else {
                assert_eq!(at, None, "{:?} from {from}", String::from_utf8_lossy(&b));
            }
            // `newline_positions`
            let want: Vec<u32> = (0..n).filter(|&i| b[i] == b'\n').map(|i| i as u32).collect();
            assert_eq!(newline_positions(&b), want);
        }
    }
}
