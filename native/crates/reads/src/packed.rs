//! A packed map of u32s by index (`Packed`): an entry a byte, its block's least value a word. The numbering's answers
//! a document's reads ask most (a numbered node's tape node and id, an id's number, a field row's node) are held so,
//! derived once at the entry's making in its block, about 1.25 bytes an entry: a tape node, an id's low bits and a
//! number are close to their neighbours'. An entry more than `ESCAPE - 1` past its block's least is the escape: its
//! reader asks the numbering, as it would with no map.
//!
//! Words: a block of 16 entries is five, its least value and then its entries' bytes (`BLOCK`), so an entry's
//! read is two loads of one line and no header: an entry past the last is an escape. A lane's entry is two gathers
//! (the join program's lanes, `lanes::packed_get`).

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

/// A packed map of values in two clusters a block (an id's low bits: a granule's slot, a hash, above its index, so
/// consecutive numbers across a child granule lie far apart): a block of 8 entries is four words, its two clusters'
/// least values and its entries' bytes, each byte's top bit its cluster and its low 7 bits the offset past it; 0xff
/// the escape. About 2 bytes an entry, one aligned 16-byte load.
pub mod two {
    use super::{ESCAPE, NONE};

    /// A block's entries.
    pub const SHIFT: u32 = 3;
    /// A block's words: its two least values, then its 8 entries' bytes.
    pub const BLOCK: usize = 4;
    /// The offsets a cluster reaches: 0..127 (127 of the second is the escape).
    const REACH: u32 = 127;

    /// How many words `build` fills for `n` entries.
    pub fn words(n: usize) -> usize {
        BLOCK * n.div_ceil(1 << SHIFT)
    }

    /// `vals` packed into `out` (`words(vals.len())` long); returns the words and how many entries escaped.
    pub fn build(vals: &[u32], out: &mut [u32]) -> (usize, usize) {
        let mut escaped = 0;
        for (b, block) in vals.chunks(1 << SHIFT).enumerate() {
            let w = &mut out[BLOCK * b..BLOCK * (b + 1)];
            let a = block.iter().copied().min().unwrap_or(0);
            let c = block.iter().copied().filter(|v| v.wrapping_sub(a) >= REACH).min().unwrap_or(a);
            (w[0], w[1]) = (a, c);
            let mut bytes = [ESCAPE; 8];
            for (k, &v) in block.iter().enumerate() {
                let (da, dc) = (v.wrapping_sub(a), v.wrapping_sub(c));
                bytes[k] = if da < REACH { da as u8 } else if dc < REACH { 0x80 | dc as u8 } else { ESCAPE };
                escaped += usize::from(bytes[k] == ESCAPE);
            }
            w[2] = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            w[3] = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        }
        (words(vals.len()), escaped)
    }

    /// Entry `i` of map `p` (`build`'s words), `NONE` for an escape or past the last, or for no map.
    #[inline(always)]
    pub fn get(p: &[u32], i: u32) -> u32 {
        let at = BLOCK * (i >> SHIFT) as usize;
        let Some(w) = p.get(at..at + BLOCK) else { return NONE };
        let byte = (w[2 + ((i >> 2) & 1) as usize] >> (8 * (i & 3))) as u8;
        if byte == ESCAPE { NONE } else { w[usize::from(byte >> 7)].wrapping_add(u32::from(byte & 0x7f)) }
    }
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

    #[test]
    fn a_two_cluster_maps_entries_are_its_values_or_escapes() {
        // Two far clusters a block (a granule's slot and its child's), and a third now and then.
        let vals: Vec<u32> = (0..1000u32).map(|i| match i % 8 { 2 | 3 => (7 << 18) + i, 5 if i % 3 == 0 => (9 << 18) + i, _ => (3 << 18) + i }).collect();
        let mut out = vec![0; super::two::words(vals.len())];
        let (w, esc) = super::two::build(&vals, &mut out);
        assert_eq!(w, out.len());
        let mut seen = 0;
        for (i, &v) in vals.iter().enumerate() {
            match super::two::get(&out, i as u32) {
                super::NONE => seen += 1,
                g => assert_eq!(g, v),
            }
        }
        assert_eq!(seen, esc);
        assert!(esc > 0 && esc <= 42, "{esc}");
        assert_eq!(super::two::get(&out, 1000), super::NONE);
    }
}
