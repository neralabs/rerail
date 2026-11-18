//! Micro-lexers: the end of one construct, found 64 bytes a step from where it starts
//! (docs/parser-generators/31-micro-lexers.md). Moved here from `rr_syntax` so every lean lexer
//! reads its strings and comments with them.
//!
//! Each is Bitlex's block shape, run from a token's start rather than over the whole file:
//!
//! - **A block is 64 bytes, a mask one bit a byte**, as Bitlex's tier 1 and simdjson make them.
//!   A block past the text's end is padded with zeros, which no micro-lexer stops at.
//! - **The masks are Bitlex's operations**: `eq` (a byte compared, all 64 at once), a byte
//!   followed by another (`find_pair`), and `escaped` (simdjson's branch-free escape finder).
//! - **No branch inside a block.** A micro-lexer makes its stop mask; the construct ends at the
//!   mask's lowest bit. The one branch a block is "no stop here yet".
//!
//! A lexer whose stage 1 would keep a mask a block only for these stops (a string's quote, a
//! comment's end) makes the mask here instead, where such a construct starts (0313's lazy masks).

use crate::{Simd8x64, dispatch};


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
unsafe fn string_end_k<S: Simd8x64>(b: &[u8], from: usize) -> (usize, bool) {
    let (mut carry, mut u) = (0, 0);
    // SAFETY: the kernel runs where the CPU runs `S` (`dispatch!`).
    let end = unsafe {
        scan::<S>(b, from, |blk, _| {
            let (esc, out) = escaped(blk.eq(b'\\'), carry);
            carry = out;
            let quote = blk.eq(b'"') & !esc;
            // `u` is looked for only in a block with an escape (most have none).
            if esc != 0 {
                u |= esc & blk.eq(b'u') & below(quote);
            }
            quote
        })
    };
    // Two words, so the call returns them in registers (`usize::MAX`: the text ends first).
    (end.unwrap_or(usize::MAX), u != 0)
}

dispatch! {
    /// The first `x` from `from` on: a line comment's end, a backtick literal's.
    pub fn find(b: &[u8], from: usize, x: u8) -> Option<usize> = find_k, on find_on;
}

dispatch! {
    /// The first `x` followed by `y` from `from` on (Bitlex's `pair`: a block comment's `*/`, the
    /// fence of a two-backtick literal).
    pub fn find_pair(b: &[u8], from: usize, x: u8, y: u8) -> Option<usize> = find_pair_k, on find_pair_on;
}

dispatch! {
    /// `string_end`'s call: the end as a word (`usize::MAX`: none).
    fn string_end_raw(b: &[u8], from: usize) -> (usize, bool) = string_end_k, on string_end_on;
}

/// A string's body from `from` (after its opening quote): its closing quote, the first not
/// escaped; and whether a `\u` escape comes before it.
///
/// A string that closes within its first 16 bytes, with no `\` before its quote, is read here
/// by word operations (most strings: a call costs more than the scan of a short one).
#[inline(always)]
pub fn string_end(b: &[u8], from: usize) -> (Option<usize>, bool) {
    if let Some(w) = b.get(from..from + 16) {
        // The bytes that are `"` and `\`, by the zero-byte test: its lowest flag is exact.
        let zero = |v: u64| v.wrapping_sub(0x0101_0101_0101_0101) & !v & 0x8080_8080_8080_8080;
        for k in [0, 8] {
            let x = u64::from_le_bytes(w[k..k + 8].try_into().expect("8 bytes"));
            let quote = zero(x ^ 0x2222_2222_2222_2222);
            let slash = zero(x ^ 0x5c5c_5c5c_5c5c_5c5c);
            if quote != 0 && quote.trailing_zeros() < slash.trailing_zeros() {
                return (Some(from + k + (quote.trailing_zeros() / 8) as usize), false);
            }
            if slash != 0 {
                break;
            }
        }
    }
    let (end, u) = string_end_raw(b, from);
    ((end != usize::MAX).then_some(end), u)
}

#[inline(always)]
unsafe fn eq_all_k<S: Simd8x64>(b: &[u8], i: usize, x: u8) -> u64 {
    // SAFETY: the kernel runs where the CPU runs `S` (`dispatch!`).
    unsafe { block::<S>(b, i).eq(x) }
}

dispatch! {
    /// The block at `i`'s `x`s (bit `k`: byte `i + k` is `x`).
    pub fn find_all(b: &[u8], i: usize, x: u8) -> u64 = eq_all_k, on find_all_on;
}

#[inline(always)]
unsafe fn first_of_k<S: Simd8x64>(b: &[u8], from: usize, set: [u8; 3]) -> usize {
    // SAFETY: the kernel runs where the CPU runs `S` (`dispatch!`).
    unsafe { scan::<S>(b, from, |blk, _| blk.eq(set[0]) | blk.eq(set[1]) | blk.eq(set[2])).map_or(b.len().max(from), |e| e.min(b.len())) }
}

dispatch! {
    /// `first_of`'s block scan.
    fn first_of_blocks(b: &[u8], from: usize, set: [u8; 3]) -> usize = first_of_k, on first_of_on;
}

/// The first byte from `from` that is one of `set` (a byte repeated for a set of one or two),
/// or the text's length: a string's or a comment's stops, made where it starts.
///
/// The first 16 bytes are looked at 8 a step by word operations, as `string_end` does: most
/// stops are that near, and a block's scan costs more than two words'.
#[inline(always)]
pub fn first_of(b: &[u8], from: usize, set: [u8; 3]) -> usize {
    if let Some(w) = b.get(from..from + 16) {
        // The high bit of each byte of `v` that is zero, exactly.
        let zero = |v: u64| !(((v & 0x7f7f_7f7f_7f7f_7f7f).wrapping_add(0x7f7f_7f7f_7f7f_7f7f)) | v | 0x7f7f_7f7f_7f7f_7f7f);
        let [x, y, z] = set.map(|c| u64::from_ne_bytes([c; 8]));
        for k in [0, 8] {
            let v = u64::from_le_bytes(w[k..k + 8].try_into().expect("8 bytes"));
            let m = zero(v ^ x) | zero(v ^ y) | zero(v ^ z);
            if m != 0 {
                return from + k + (m.trailing_zeros() / 8) as usize;
            }
        }
        return first_of_blocks(b, from + 16, set);
    }
    first_of_blocks(b, from, set)
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
    fn backends() -> Vec<crate::Backend> {
        crate::Backend::ALL.into_iter().filter(|b| b.runs()).collect()
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
                assert_eq!(string_end_on(be, &b, from), Some(string_end_raw(&b, from)), "{be:?}");
                let (end, u) = string_end_raw(&b, from);
                assert_eq!(string_end(&b, from), ((end != usize::MAX).then_some(end), u));
            }
            let (at, got_u) = string_end(&b, from);
            if end < n {
                assert_eq!(at, Some(end), "{:?} from {from}", String::from_utf8_lossy(&b));
                assert_eq!(got_u, u);
            } else {
                assert_eq!(at, None, "{:?} from {from}", String::from_utf8_lossy(&b));
            }
            // `first_of`, of one, two and three bytes
            for set in [[b'`'; 3], [b'*', b'/', b'/'], [b'"', b'\\', b'\n']] {
                let want = b[from..].iter().position(|c| set.contains(c)).map_or(n.max(from), |e| from + e);
                assert_eq!(first_of(&b, from, set), want, "{set:?} {:?} from {from}", String::from_utf8_lossy(&b));
                for be in backends() {
                    assert_eq!(first_of_on(be, &b, from, set), Some(want), "{be:?}");
                }
            }
        }
    }
}
