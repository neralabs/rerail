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

/// 64 bytes in a backend's registers.
struct Block<S: Simd8x64>(S::V);

impl<S: Simd8x64> Block<S> {
    /// Bit `i`: byte `i` is `x`.
    #[inline(always)]
    unsafe fn eq(&self, x: u8) -> u64 {
        // SAFETY: the kernel runs where the CPU runs `S` (`dispatch!`).
        unsafe { S::eq_mask(self.0, x) }
    }
}

/// The lexer's four masks over the 64 bytes `p[..64]`; `p[64..]` are the bytes after, for the
/// pairs' second bytes (read as the 64 bytes from `p[1]`).
#[inline(always)]
unsafe fn lexer<S: Simd8x64>(p: &[u8; 80]) -> [u64; 4] {
    // SAFETY: the kernel runs where the CPU runs `S` (`dispatch!`).
    unsafe {
        let x = S::load(p[..64].try_into().expect("64 bytes"));
        let d = S::load(p[1..65].try_into().expect("64 bytes"));
        let c = S::classify(x, &LO, &HI);
        let space = S::nonzero(S::and(c, S::splat(SPACE)));
        let word = S::nonzero(S::and(c, S::splat(WORD)));
        // A non-ASCII character's first byte (`0xc0..`): its high nibble.
        let rare = S::nonzero(S::and(c, S::splat(RARE))) | S::nonzero(S::lookup_16(S::high_nibbles(x), &LEAD));
        // A pair: the first byte's table names the second bytes it takes, the next byte's which
        // it is.
        let pair = S::nonzero(S::and(S::classify(x, &FIRST_LO, &FIRST_HI), S::classify(d, &SECOND_LO, &SECOND_HI)));
        [space, word, rare, pair]
    }
}

dispatch! {
    /// The lexer's four masks (`Window`).
    fn lexer_masks(p: &[u8; 80]) -> [u64; 4] = lexer, on lexer_masks_on;
}

/// The high nibbles of a non-ASCII character's first byte.
const LEAD: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1];

/// The block at `i`: 64 bytes, zeros past the end.
#[inline(always)]
unsafe fn block<S: Simd8x64>(b: &[u8], i: usize) -> Block<S> {
    // SAFETY: the kernel runs where the CPU runs `S` (`dispatch!`).
    unsafe {
        match b.get(i..i + 64) {
            Some(s) => Block(S::load(s.try_into().expect("64 bytes"))),
            None => {
                let mut pad = [0u8; 64];
                pad[..b.len() - i].copy_from_slice(&b[i..]);
                Block(S::load(&pad))
            }
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

#[cfg(test)]
#[inline(always)]
unsafe fn find_all_k<S: Simd8x64>(b: &[u8], i: usize, x: u8) -> u64 {
    // SAFETY: the kernel runs where the CPU runs `S` (`dispatch!`).
    unsafe { block::<S>(b, i).eq(x) }
}

#[cfg(test)]
dispatch! {
    /// The block at `i`'s `x`s.
    fn find_all(b: &[u8], i: usize, x: u8) -> u64 = find_all_k, on find_all_on;
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
unsafe fn scan<S: Simd8x64>(b: &[u8], from: usize, mut stop: impl FnMut(&Block<S>, usize) -> u64) -> Option<usize> {
    let mut i = from;
    while i < b.len() {
        // SAFETY: the kernel runs where the CPU runs `S` (`dispatch!`).
        let s = stop(&unsafe { block::<S>(b, i) }, i);
        if s != 0 {
            return Some(i + s.trailing_zeros() as usize);
        }
        i += 64;
    }
    None
}

#[inline(always)]
unsafe fn find_k<S: Simd8x64>(b: &[u8], from: usize, x: u8) -> Option<usize> {
    // SAFETY: the kernel runs where the CPU runs `S` (`dispatch!`).
    unsafe { scan::<S>(b, from, |blk, _| blk.eq(x)) }
}

#[inline(always)]
unsafe fn find_pair_k<S: Simd8x64>(b: &[u8], from: usize, x: u8, y: u8) -> Option<usize> {
    // SAFETY: the kernel runs where the CPU runs `S` (`dispatch!`).
    unsafe {
        scan::<S>(b, from, |blk, i| {
            // The next block's first byte, for this block's last.
            let next = u64::from(b.get(i + 64) == Some(&y));
            blk.eq(x) & ((blk.eq(y) >> 1) | (next << 63))
        })
    }
}

#[inline(always)]
unsafe fn string_end_k<S: Simd8x64>(b: &[u8], from: usize) -> (Option<usize>, bool) {
    let (mut carry, mut u) = (0, 0);
    // SAFETY: the kernel runs where the CPU runs `S` (`dispatch!`).
    let end = unsafe {
        scan::<S>(b, from, |blk, _| {
            let (esc, out) = escaped(blk.eq(b'\\'), carry);
            carry = out;
            let quote = blk.eq(b'"') & !esc;
            u |= esc & blk.eq(b'u') & below(quote);
            quote
        })
    };
    (end, u != 0)
}

dispatch! {
    /// The first `x` from `from` on: a line comment's end, a backtick literal's.
    pub(crate) fn find(b: &[u8], from: usize, x: u8) -> Option<usize> = find_k, on find_on;
}

dispatch! {
    /// The first `x` followed by `y` from `from` on (Bitlex's `pair`: a block comment's `*/`, the
    /// fence of a two-backtick literal).
    pub(crate) fn find_pair(b: &[u8], from: usize, x: u8, y: u8) -> Option<usize> = find_pair_k, on find_pair_on;
}

dispatch! {
    /// A string's body from `from` (after its opening quote): its closing quote, the first not
    /// escaped; and whether a `\u` escape comes before it.
    pub(crate) fn string_end(b: &[u8], from: usize) -> (Option<usize>, bool) = string_end_k, on string_end_on;
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
            for be in backends() {
                assert_eq!(find_on(be, &b, from, b'`'), Some(find(&b, from, b'`')), "{be:?}");
                assert_eq!(find_pair_on(be, &b, from, b'*', b'/'), Some(find_pair(&b, from, b'*', b'/')), "{be:?}");
                assert_eq!(string_end_on(be, &b, from), Some(string_end(&b, from)), "{be:?}");
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
