//! A packed map of u32s by index (`Packed`): an entry a byte, its block's least value a word. The numbering's answer
//! a document's reads ask most (a numbered node's tape node, its id under tape ids) is held so, derived once at the
//! entry's making in its block, about 1.25 bytes an entry: a tape node is close to its neighbours'. An
//! entry more than `ESCAPE - 1` past its block's least is the escape: its reader asks the numbering, as with no map.
//!
//! Words: a block of 16 entries is five, its least value and then its entries' bytes (`BLOCK`), so an entry's
//! read is two loads of one line and no header: an entry past the last is an escape.

/// The byte of an entry its block's least value does not reach: read from the numbering.
pub const ESCAPE: u8 = 0xff;
/// No entry (an escape, or past the last).
pub const NONE: u32 = u32::MAX;
/// A block's entries.
pub const SHIFT: u32 = 4;
/// A block's words: its least value, then its 16 entries' bytes.
pub const BLOCK: usize = 5;

/// How many words `build` fills for `n` entries.
pub fn words(n: usize) -> usize {
    BLOCK * n.div_ceil(1 << SHIFT)
}

/// `vals` packed into `out` (`words(vals.len())` long); returns the words and how many entries escaped.
pub fn build(vals: &[u32], out: &mut [u32]) -> (usize, usize) {
    let mut escaped = 0;
    for (b, block) in vals.chunks(1 << SHIFT).enumerate() {
        let w = &mut out[BLOCK * b..BLOCK * (b + 1)];
        let base = block.iter().copied().min().unwrap_or(0);
        w[0] = base;
        let mut bytes = [ESCAPE; 16];
        for (k, &v) in block.iter().enumerate() {
            let d = v.wrapping_sub(base);
            bytes[k] = if d < u32::from(ESCAPE) { d as u8 } else { ESCAPE };
            escaped += usize::from(bytes[k] == ESCAPE);
        }
        for q in 0..4 {
            w[1 + q] = u32::from_le_bytes([bytes[4 * q], bytes[4 * q + 1], bytes[4 * q + 2], bytes[4 * q + 3]]);
        }
    }
    (words(vals.len()), escaped)
}

/// Entry `i` of packed map `p` (`build`'s words), `NONE` for an escape or past the last, or for no map.
#[inline(always)]
pub fn get(p: &[u32], i: u32) -> u32 {
    let at = BLOCK * (i >> SHIFT) as usize;
    let Some(w) = p.get(at..at + BLOCK) else { return NONE };
    let byte = (w[1 + ((i >> 2) & 3) as usize] >> (8 * (i & 3))) as u8;
    if byte == ESCAPE { NONE } else { w[0].wrapping_add(u32::from(byte)) }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_packed_maps_entries_are_its_values_or_escapes() {
        let vals: Vec<u32> = (0..1000u32).map(|i| if i % 97 == 5 { 1_000_000 + i } else { 3 * i + (i % 7) }).collect();
        let mut out = vec![0; super::words(vals.len())];
        let (w, esc) = super::build(&vals, &mut out);
        assert_eq!(w, out.len());
        let mut seen = 0;
        for (i, &v) in vals.iter().enumerate() {
            match super::get(&out, i as u32) {
                super::NONE => seen += 1,
                g => assert_eq!(g, v),
            }
        }
        assert_eq!(seen, esc);
        assert!(esc > 0 && esc < 40);
        assert_eq!(super::get(&out, 1000), super::NONE);
        assert_eq!(super::get(&out, 1008), super::NONE);

        assert_eq!(super::get(&[], 0), super::NONE);
    }
}
