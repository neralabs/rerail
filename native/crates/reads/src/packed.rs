//! A packed map of u32s by index (`Packed`): an entry a byte, its block's least value a word. The numbering's answers
//! a document's reads ask most (a numbered node's tape node and id, an id's number, a field row's node) are held so,
//! derived once at the entry's making in its block, about 1.25 bytes an entry: a tape node, an id's low bits and a
//! number are close to their neighbours'. An entry more than `ESCAPE - 1` past its block's least is the escape: its
//! reader asks the numbering, as it would with no map.
//!
//! Words: the entries, the block's shift, then a block's least value each, then the entries' bytes.

/// The byte of an entry its block's least value does not reach: read from the numbering.
pub const ESCAPE: u8 = 0xff;
/// No entry (an escape, or past the last).
pub const NONE: u32 = u32::MAX;
/// The blocks' entries: 16, a block's least value a word.
pub const SHIFT: u32 = 4;

/// How many words `build` fills for `n` entries.
pub fn words(n: usize) -> usize {
    2 + n.div_ceil(1 << SHIFT) + n.div_ceil(4)
}

/// `vals` packed into `out` (`words(vals.len())` long); returns the words and how many entries escaped.
pub fn build(vals: &[u32], out: &mut [u32]) -> (usize, usize) {
    let n = vals.len();
    let nb = n.div_ceil(1 << SHIFT);
    out[0] = n as u32;
    out[1] = SHIFT;
    let mut escaped = 0;
    let (bases, bytes) = out[2..].split_at_mut(nb);
    for (b, base) in bases.iter_mut().enumerate() {
        let block = &vals[b << SHIFT..((b + 1) << SHIFT).min(n)];
        *base = block.iter().copied().min().unwrap_or(0);
    }
    for w in bytes.iter_mut() {
        *w = 0;
    }
    for (i, &v) in vals.iter().enumerate() {
        let d = v.wrapping_sub(bases[i >> SHIFT]);
        let byte = if d < u32::from(ESCAPE) { d as u8 } else { ESCAPE };
        escaped += usize::from(byte == ESCAPE);
        bytes[i / 4] |= u32::from(byte) << (8 * (i % 4));
    }
    (words(n), escaped)
}

/// Entry `i` of packed map `p` (`build`'s words), `NONE` for an escape or past the last, or for no map.
#[inline(always)]
pub fn get(p: &[u32], i: u32) -> u32 {
    let Some(&n) = p.first() else { return NONE };
    if i >= n {
        return NONE;
    }
    let nb = n.div_ceil(1 << SHIFT) as usize;
    let byte = (p[2 + nb + (i / 4) as usize] >> (8 * (i % 4))) as u8;
    if byte == ESCAPE { NONE } else { p[2 + (i >> SHIFT) as usize].wrapping_add(u32::from(byte)) }
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
        assert_eq!(super::get(&[], 0), super::NONE);
    }
}
