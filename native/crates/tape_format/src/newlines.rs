//! A document's newline index, part of its format 16 tape (decision 0156 §1.6, 0336 §1): the offsets of its
//! source's `\n` bytes in Elias–Fano form. A node's line is the rank of its start among them (`tape_line`); a line's
//! first byte is the one past the newline before it (`tape_line_start`). The reads are the tape's, written once in
//! CLIF (`rerail_clif::reads`); this module writes the index and says where its parts lie.
//!
//! The index, all little-endian:
//!
//! - five u32s: the newlines' count `m`, the low parts' width `L` in bits, the zero samples' count, the one samples'
//!   count, and the high bits' length in u64 words;
//! - the zero samples, then the one samples: for every `SAMPLE`-th zero (one) of the high bits, a u32 pair: the word
//!   it lies in, and how many zeros (ones) the words before it hold;
//! - the high bits: newline `i`'s offset `o` sets bit `(o >> L) + i`; bucket `h`'s newlines are the ones before the
//!   `h`-th zero;
//! - the low parts: newline `i`'s low `L` bits at bit `i * L`.
//!
//! A word's load past the low parts reads the region's 8 bytes of room.

/// Every `SAMPLE`-th zero and one of the high bits is sampled.
pub const SAMPLE_SHIFT: u32 = 8;
pub const SAMPLE: usize = 1 << SAMPLE_SHIFT;
/// The index's first fields, u32s: `m`, `L`, zero samples, one samples, high words.
pub const HEAD: usize = 20;

/// The index of `source`'s newlines: its bytes, without the room a region keeps after them.
pub fn index(source: &[u8]) -> Vec<u8> {
    let offsets: Vec<u64> = memchr::memchr_iter(b'\n', source).map(|at| at as u64).collect();
    let (m, u) = (offsets.len() as u64, source.len() as u64);
    let l = if m == 0 || u <= m { 0 } else { 63 - (u / m).leading_zeros() };
    let buckets = (u >> l) + 1;
    let bits = m + buckets;
    let words = bits.div_ceil(64) as usize;
    let mut high = vec![0u64; words];
    for (i, &o) in offsets.iter().enumerate() {
        let at = (o >> l) + i as u64;
        high[(at / 64) as usize] |= 1 << (at % 64);
    }
    // Samples: the word holding every SAMPLE-th zero (one), and the zeros (ones) before that word.
    let samples = |ones: bool, count: u64| {
        let mut out = Vec::with_capacity(count.div_ceil(SAMPLE as u64) as usize);
        let mut before = 0u64;
        for (w, &word) in high.iter().enumerate() {
            let valid = if (w + 1) * 64 <= bits as usize { 64 } else { bits as usize - w * 64 };
            let set = (word.count_ones()) as u64;
            let here = if ones { set } else { valid as u64 - set };
            while (out.len() as u64) * (SAMPLE as u64) < before + here && (out.len() as u64) * (SAMPLE as u64) < count {
                out.push((w as u32, before as u32));
            }
            before += here;
        }
        out
    };
    let zeros = samples(false, buckets);
    let ones = samples(true, m);
    let mut out = Vec::with_capacity(HEAD + 8 * (zeros.len() + ones.len() + words) + (m * u64::from(l)).div_ceil(8) as usize);
    for v in [m as u32, l, zeros.len() as u32, ones.len() as u32, words as u32] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for (w, before) in zeros.iter().chain(&ones) {
        out.extend_from_slice(&w.to_le_bytes());
        out.extend_from_slice(&before.to_le_bytes());
    }
    for word in &high {
        out.extend_from_slice(&word.to_le_bytes());
    }
    let mut low = vec![0u8; (m * u64::from(l)).div_ceil(8) as usize + 8];
    if l > 0 {
        let mask = (1u64 << l) - 1;
        for (i, &o) in offsets.iter().enumerate() {
            let bit = i as u64 * u64::from(l);
            let at = (bit / 8) as usize;
            let mut word = u64::from_le_bytes(low[at..at + 8].try_into().expect("8 bytes"));
            word |= (o & mask) << (bit % 8);
            low[at..at + 8].copy_from_slice(&word.to_le_bytes());
        }
    }
    low.truncate((m * u64::from(l)).div_ceil(8) as usize);
    out.extend_from_slice(&low);
    out
}

/// The newlines' count an index holds (a document has one line more).
pub fn count(index: &[u8]) -> Option<u32> {
    Some(u32::from_le_bytes(index.get(..4)?.try_into().ok()?))
}

/// The index checked against itself and its source's length: its fields' sizes add up to its length, and its
/// buckets cover the source.
pub fn check(index: &[u8], source_len: u64) -> Result<(), String> {
    let field = |k: usize| index.get(4 * k..4 * k + 4).map(|b| u64::from(u32::from_le_bytes(b.try_into().expect("4 bytes"))));
    let bad = || "a newline index that does not fit its tape".to_owned();
    let (m, l, zs, os, words) = (field(0).ok_or_else(bad)?, field(1).ok_or_else(bad)?, field(2).ok_or_else(bad)?, field(3).ok_or_else(bad)?, field(4).ok_or_else(bad)?);
    if l > 32 || m > source_len {
        return Err(bad());
    }
    let buckets = (source_len >> l) + 1;
    let len = HEAD as u64 + 8 * (zs + os + words) + (m * l).div_ceil(8);
    if len != index.len() as u64 || words != (m + buckets).div_ceil(64) || zs != buckets.div_ceil(SAMPLE as u64) || os != m.div_ceil(SAMPLE as u64) {
        return Err(bad());
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Sources whose newlines are dense, sparse, bunched and absent.
    pub(crate) fn sources() -> Vec<Vec<u8>> {
        let mut out = vec![Vec::new(), b"no newline".to_vec(), b"\n".to_vec(), b"a\nb\n".to_vec(), vec![b'\n'; 700]];
        let mut long = Vec::new();
        for k in 0..3_000 {
            long.extend(std::iter::repeat_n(b'x', (k * 37) % 150));
            long.push(b'\n');
            if k % 400 == 0 {
                long.extend(vec![b'\n'; 300]);
            }
        }
        out.push(long);
        out
    }

    #[test]
    fn every_index_checks_against_its_source() {
        for s in sources() {
            let index = index(&s);
            check(&index, s.len() as u64).unwrap();
            assert_eq!(count(&index), Some(s.iter().filter(|&&b| b == b'\n').count() as u32));
            assert!(check(&index[..index.len() - 1], s.len() as u64).is_err(), "a short index");
        }
    }
}
