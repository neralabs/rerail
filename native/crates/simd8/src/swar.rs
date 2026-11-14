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
        let x = match src.get(i..i + 8) {
            Some(w) => u64::from_le_bytes(w.try_into().expect("8")),
            None => tail(src, i),
        };
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

/// The 8 bytes at `i` within 8 of the text's end, zeros past it.
#[cold]
#[inline(never)]
fn tail(src: &[u8], i: usize) -> u64 {
    let mut w = [0u8; 8];
    let n = src.len() - i;
    w[..n].copy_from_slice(&src[i..]);
    u64::from_le_bytes(w)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
