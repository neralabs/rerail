//! Byte searches within a short range, 8 bytes a step (SWAR: one `u64` a step, no per-byte loop),
//! for the scans stage 1's masks do not cover: a range a lexer has already bounded by a mask (an
//! interpolated string's text up to its next quote or line end), too rare to give a mask of its own
//! over every block.

const LO7: u64 = 0x7f7f_7f7f_7f7f_7f7f;

/// The high bit of each byte of `v` that is zero, exactly (no borrow from a lower byte).
#[inline(always)]
const fn zero_bytes(v: u64) -> u64 {
    !(((v & LO7).wrapping_add(LO7)) | v | LO7)
}

/// The first byte of `src[from..to]` that is `a` or `b`; `to` where none is.
#[inline]
pub fn first_of2(src: &[u8], from: usize, to: usize, a: u8, b: u8) -> usize {
    let to = to.min(src.len());
    let (sa, sb) = (u64::from_ne_bytes([a; 8]), u64::from_ne_bytes([b; 8]));
    let mut i = from;
    while i < to {
        let x = load8(src, i);
        let mut m = zero_bytes(x ^ sa) | zero_bytes(x ^ sb);
        // Bytes at or past `to` are not the range's.
        if to - i < 8 {
            m &= (1u64 << (8 * (to - i))) - 1;
        }
        if m != 0 {
            return i + (m.trailing_zeros() / 8) as usize;
        }
        i += 8;
    }
    to
}

/// The end of the run of byte `c` from `from`: the first byte that is not `c`, or the text's end.
#[inline]
pub fn run_end(src: &[u8], from: usize, c: u8) -> usize {
    let sc = u64::from_ne_bytes([c; 8]);
    let mut i = from;
    while i < src.len() {
        // The bytes that differ from `c`; past the text's end, all do.
        let mut m = !zero_bytes(load8(src, i) ^ sc) & 0x8080_8080_8080_8080;
        if src.len() - i < 8 {
            m |= !((1u64 << (8 * (src.len() - i))) - 1) & 0x8080_8080_8080_8080;
        }
        if m != 0 {
            return i + (m.trailing_zeros() / 8) as usize;
        }
        i += 8;
    }
    src.len().max(from)
}

/// A byte class for `class_end`: one flag a byte value.
pub type Class = [bool; 256];

/// The class of the bytes in `set`, as a table (for a `static`).
pub const fn class_of(set: &[u8]) -> Class {
    let mut t = [false; 256];
    let mut i = 0;
    while i < set.len() {
        t[set[i] as usize] = true;
        i += 1;
    }
    t
}

/// The end of the run of bytes in class `class` from `from`: the first byte outside it, or the
/// text's end. Each 8 bytes at once: 8 straight-line lookups, a lane each.
#[inline]
pub fn class_end(src: &[u8], from: usize, class: &Class) -> usize {
    let mut i = from;
    while i < src.len() {
        let x = load8(src, i);
        let lane = |k: u32| u64::from(!class[(x >> (8 * k)) as u8 as usize]) << (8 * k);
        let mut m = lane(0) | lane(1) | lane(2) | lane(3) | lane(4) | lane(5) | lane(6) | lane(7);
        if src.len() - i < 8 {
            m |= !((1u64 << (8 * (src.len() - i))) - 1) & 0x0101_0101_0101_0101;
        }
        if m != 0 {
            return i + (m.trailing_zeros() / 8) as usize;
        }
        i += 8;
    }
    src.len().max(from)
}

/// The 8 bytes at `i` (little-endian: byte `i` the lowest), zeros past the text's end (all of
/// them for an `i` past it).
#[inline(always)]
pub fn load8(src: &[u8], i: usize) -> u64 {
    // One compare in a loop: `src.len() >= 8` is the loop's, and `i + 8` cannot overflow here.
    if src.len() >= 8 && i <= src.len() - 8 {
        // SAFETY: `i + 8 <= src.len()`.
        u64::from_le(unsafe { src.as_ptr().add(i).cast::<u64>().read_unaligned() })
    } else {
        tail(src, i)
    }
}

/// The 8 bytes at `i` within 8 of the text's end (or past it), zeros past it.
#[cold]
#[inline(never)]
fn tail(src: &[u8], i: usize) -> u64 {
    let mut w = [0u8; 8];
    let n = src.len().saturating_sub(i);
    if n > 0 {
        w[..n].copy_from_slice(&src[i..]);
    }
    u64::from_le_bytes(w)
}

/// Whether the first `n` bytes of `w` (`n` up to 8; byte 0 the lowest) are all ASCII digits: a
/// number's digits tested 8 at a time.
#[inline(always)]
pub fn digits(w: u64, n: usize) -> bool {
    const LOW: u64 = 0x7f7f_7f7f_7f7f_7f7f;
    const HIGH: u64 = 0x8080_8080_8080_8080;
    debug_assert!(n <= 8);
    let y = w ^ 0x3030_3030_3030_3030;
    // A byte's high bit: it is past 0x7f after the xor, or its low seven bits are 10 or more.
    let bad = (((y & LOW) + 0x7676_7676_7676_7676) | y) & HIGH;
    let keep = if n >= 8 { !0 } else { (1u64 << (8 * n)) - 1 };
    bad & keep == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_against_each_byte() {
        let mut x = 0x2545_f491_4f6c_dd1du64;
        for _ in 0..20_000 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            // Bytes near the digits, and any byte.
            let w = if x & 1 == 0 { (x & 0x0f0f_0f0f_0f0f_0f0f) | 0x3030_3030_3030_3030 } else { x };
            for n in 0..=8 {
                let want = w.to_le_bytes()[..n].iter().all(u8::is_ascii_digit);
                assert_eq!(digits(w, n), want, "{w:#x} {n}");
            }
        }
    }

    #[test]
    fn the_first_of_two_bytes_in_a_range() {
        let s = b"abc{def}ghij{k";
        for from in 0..s.len() {
            for to in from..=s.len() + 2 {
                let want = (from..to.min(s.len())).find(|&i| s[i] == b'{' || s[i] == b'}').unwrap_or(to.min(s.len()));
                assert_eq!(first_of2(s, from, to, b'{', b'}'), want, "{from}..{to}");
            }
        }
        // A byte one past the searched one (0x7c, 0x7e) and a high byte are not found.
        assert_eq!(first_of2(b"|~\xfb\xfdxyz", 0, 7, b'{', b'}'), 7);
    }

    #[test]
    fn runs_and_class_runs_end_where_a_byte_test_says() {
        let s = b"__a___b________________c_x--x-";
        let dash = class_of(b"-_abc");
        for from in 0..=s.len() {
            let want = (from..s.len()).find(|&i| s[i] != b'_').unwrap_or(s.len());
            assert_eq!(run_end(s, from, b'_'), want, "run from {from}");
            let want = (from..s.len()).find(|&i| !dash[s[i] as usize]).unwrap_or(s.len());
            assert_eq!(class_end(s, from, &dash), want, "class from {from}");
        }
        // A high byte, and 0 past the end, are outside a class that holds neither.
        assert_eq!(class_end(b"ab\xffc", 0, &dash), 2);
        assert_eq!(run_end(b"\0\0", 0, 0), 2);
    }
}
