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
//! The masks come from `rerail-simd8` (simdjson's `simd8x64`): NEON on aarch64, AVX2 or SSE4.2 on
//! x86_64 (the best the CPU runs, chosen at run time), and 8 bytes a word elsewhere. Each
//! micro-lexer is a kernel over `Simd8x64`, compiled for each backend (`dispatch!`); a test checks
//! every backend the CPU runs against the byte tests.

use rerail_simd8::{Simd8x64, dispatch};
pub(crate) use rerail_simd8::micro::{find, find_pair, string_end};
#[cfg(test)]
use rerail_simd8::micro::{find_all, find_all_on};

/// The lexer's four masks over the 64 bytes `p[..64]`; `p[64]` is the byte after, the last
/// pair's second byte.
#[inline(always)]
unsafe fn lexer<S: Simd8x64>(p: &[u8; 80]) -> [u64; 4] {
    // SAFETY: the kernel runs where the CPU runs `S` (`dispatch!`).
    unsafe {
        let x = S::load(p[..64].try_into().expect("64 bytes"));
        let d = S::shift_in(x, p[64]);
        let c = S::classify(x, &LO, &HI);
        // Each mask one test, packed once (`test` and `ge` stay vectors).
        let space = S::to_bitmask(S::test(c, S::splat(SPACE)));
        let word = S::to_bitmask(S::test(c, S::splat(WORD)));
        // Or a non-ASCII character's first byte (`0xc0..`).
        let rare = S::to_bitmask(S::or(S::test(c, S::splat(RARE)), S::ge(x, 0xc0)));
        // A pair: the first byte's table names the second bytes it takes, the next byte's which
        // it is.
        let pair = S::to_bitmask(S::test(S::classify(x, &FIRST_LO, &FIRST_HI), S::classify(d, &SECOND_LO, &SECOND_HI)));
        [space, word, rare, pair]
    }
}

dispatch! {
    /// The lexer's four masks (`Window`): a step of the lexer's loop, inlined.
    #[inline(always)]
    fn lexer_masks(p: &[u8; 80]) -> [u64; 4] = lexer, on lexer_masks_on;
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
        // The masks from the text itself where it holds the 80 bytes (not from the copy: its
        // stores would be loaded again).
        match b.get(base..base + 80) {
            Some(s) => {
                let s: &[u8; 80] = s.try_into().expect("80 bytes");
                let [space, word, rare, pair] = lexer_masks(s);
                Window { base, space, word, rare, pair, bytes: *s }
            }
            None => Self::padded(b, base),
        }
    }

    /// The window at the text's end, its bytes padded with zeros: once a text, so called (the
    /// lexer's loop holds one copy of the kernel).
    #[cold]
    #[inline(never)]
    fn padded(b: &[u8], base: usize) -> Self {
        let mut bytes = [0u8; 80];
        let s = &b[base..];
        bytes[..s.len()].copy_from_slice(s);
        let [space, word, rare, pair] = lexer_masks(&bytes);
        Window { base, space, word, rare, pair, bytes }
    }
}

/// Where each newline of `b` is: a block's `eq` mask, its bits taken one at a time.
#[cfg(test)]
pub(crate) fn newline_positions(b: &[u8]) -> Vec<u32> {
    let mut out = Vec::with_capacity(b.len() / 32);
    let mut i = 0;
    while i < b.len() {
        let mut m = find_all(b, i, b'\n');
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

    /// The backends this CPU runs.
    fn backends() -> Vec<rerail_simd8::Backend> {
        rerail_simd8::Backend::ALL.into_iter().filter(|b| b.runs()).collect()
    }

    #[test]
    fn masks_are_the_byte_compares() {
        let mut r = Rng(0x9e37_79b9_7f4a_7c15);
        for _ in 0..2000 {
            let t: Vec<u8> = (0..64).map(|_| r.next() as u8 % 8).collect();
            let x = r.next() as u8 % 8;
            let want = (0..64).fold(0u64, |m, i| m | (u64::from(t[i] == x) << i));
            assert_eq!(find_all(&t, 0, x), want);
            for be in backends() {
                assert_eq!(find_all_on(be, &t, 0, x), Some(want), "{be:?}");
            }
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
            for be in backends() {
                assert_eq!(lexer_masks_on(be, &pad), Some([w.space, w.word, w.rare, w.pair]), "{be:?}");
            }
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
    fn newline_positions_are_the_newlines() {
        let mut r = Rng(0x2545_f491_4f6c_dd1d);
        for _ in 0..2000 {
            let n = (r.next() % 200) as usize;
            let b = r.text(n);
            let want: Vec<u32> = (0..n).filter(|&i| b[i] == b'\n').map(|i| i as u32).collect();
            assert_eq!(newline_positions(&b), want);
        }
    }
}
