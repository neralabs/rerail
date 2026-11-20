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
    let mut out = Vec::new();
    index_into(&mut out, source);
    out
}

/// `index`, into `out` (its bytes replaced): a scan of the source for its newlines.
pub fn index_into(out: &mut Vec<u8>, source: &[u8]) {
    let m = memchr::memchr_iter(b'\n', source).count() as u64;
    index_of(out, source.len() as u64, m, memchr::memchr_iter(b'\n', source).map(|at| at as u64));
}

/// The index of `m` newlines at `offsets` (ascending) in a source of `u` bytes, into `out` (its bytes replaced, its
/// room kept): made in place, one pass over the offsets, so a producer that has the newlines (tier 1's bits) needs no
/// scan of the source.
pub fn index_of(out: &mut Vec<u8>, u: u64, m: u64, offsets: impl Iterator<Item = u64>) {
    let l = if m == 0 || u <= m { 0 } else { 63 - (u / m).leading_zeros() };
    let buckets = (u >> l) + 1;
    let bits = m + buckets;
    let words = bits.div_ceil(64) as usize;
    let (zs, os) = (buckets.div_ceil(SAMPLE as u64) as usize, m.div_ceil(SAMPLE as u64) as usize);
    let low_len = (m * u64::from(l)).div_ceil(8) as usize;
    let (high_at, low_at) = (HEAD + 8 * (zs + os), HEAD + 8 * (zs + os + words));
    out.clear();
    out.resize(low_at + low_len + 8, 0);
    let store = |out: &mut [u8], at: usize, word: u64| out[at..at + 8].copy_from_slice(&word.to_le_bytes());
    // The high bits a word at a time and the low parts a word of bits at a time, each stored once it is whole: the
    // offsets ascend, so every bit of a word comes before the next word's.
    let mask = if l > 0 { (1u64 << l) - 1 } else { 0 };
    let (mut word, mut w_at) = (0u64, 0usize);
    let (mut low, mut low_bits, mut low_byte) = (0u64, 0u32, low_at);
    let mut n = 0u64;
    for o in offsets {
        let at = (o >> l) + n;
        let w = (at / 64) as usize;
        if w != w_at {
            store(out, high_at + 8 * w_at, word);
            (word, w_at) = (0, w);
        }
        word |= 1 << (at % 64);
        if l > 0 {
            let part = o & mask;
            low |= part << low_bits;
            low_bits += l;
            if low_bits >= 32 {
                // Four whole bytes out; the bits past them kept (`l` is at most 32).
                out[low_byte..low_byte + 4].copy_from_slice(&(low as u32).to_le_bytes());
                (low, low_bits, low_byte) = (low >> 32, low_bits - 32, low_byte + 4);
            }
        }
        n += 1;
    }
    debug_assert_eq!(n, m, "the newlines counted are the offsets given");
    if n > 0 {
        store(out, high_at + 8 * w_at, word);
    }
    store(out, low_byte, low);
    out.truncate(low_at + low_len);
    // Samples, in one pass over the high words: the word holding every SAMPLE-th zero (one), and the zeros (ones)
    // before that word.
    let word_at = |out: &[u8], at: usize| u64::from_le_bytes(out[at..at + 8].try_into().expect("8 bytes"));
    let (mut kz, mut ko, mut zeros, mut ones) = (0u64, 0u64, 0u64, 0u64);
    let sample = SAMPLE as u64;
    for w in 0..words {
        let x = word_at(out, high_at + 8 * w);
        let valid = if (w + 1) * 64 <= bits as usize { 64 } else { bits as u64 - 64 * w as u64 };
        let set = u64::from(x.count_ones());
        let (z_here, o_here) = (valid - set, set);
        while kz * sample < zeros + z_here && kz * sample < buckets {
            let at = HEAD + 8 * kz as usize;
            out[at..at + 4].copy_from_slice(&(w as u32).to_le_bytes());
            out[at + 4..at + 8].copy_from_slice(&(zeros as u32).to_le_bytes());
            kz += 1;
        }
        while ko * sample < ones + o_here && ko * sample < m {
            let at = HEAD + 8 * (zs + ko as usize);
            out[at..at + 4].copy_from_slice(&(w as u32).to_le_bytes());
            out[at + 4..at + 8].copy_from_slice(&(ones as u32).to_le_bytes());
            ko += 1;
        }
        zeros += z_here;
        ones += o_here;
    }
    debug_assert_eq!((kz as usize, ko as usize), (zs, os), "the samples' count");
    for (k, v) in [m as u32, l, zs as u32, os as u32, words as u32].into_iter().enumerate() {
        out[4 * k..4 * k + 4].copy_from_slice(&v.to_le_bytes());
    }
}

/// The offsets of the set bits of a bitmap of `u` bytes, bit `i + delay` of word `w` (`word(w)`) byte `64 w + i`: a
/// producer's newline bits as `index_of` takes them, and their count.
pub fn bits_of(u: u64, delay: u32, words: usize, word: impl Fn(usize) -> u64 + Copy) -> (u64, impl Iterator<Item = u64>) {
    let d = u64::from(delay);
    let inside = move |w: usize| {
        let x = word(w);
        // Bits before the source (the delay's) and past its end are none of its bytes.
        let lo = if w == 0 { !0u64 << d.min(63) & if d >= 64 { 0 } else { !0 } } else { !0 };
        let end = u + d;
        let hi = if (w as u64 + 1) * 64 <= end { !0 } else if (w as u64) * 64 >= end { 0 } else { (1u64 << (end - w as u64 * 64)) - 1 };
        x & lo & hi
    };
    let m = (0..words).map(|w| u64::from(inside(w).count_ones())).sum();
    let offsets = (0..words).flat_map(move |w| {
        let mut x = inside(w);
        std::iter::from_fn(move || {
            (x != 0).then(|| {
                let b = u64::from(x.trailing_zeros());
                x &= x - 1;
                w as u64 * 64 + b - d
            })
        })
    });
    (m, offsets)
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

    /// The index from a bitmap of the newlines (`bits_of`, at any delay) is the source scan's, byte for byte.
    #[test]
    fn an_index_from_bits_is_the_scans() {
        for s in sources() {
            let want = index(&s);
            for delay in [0u32, 1, 5, 63] {
                let total = s.len() + delay as usize;
                let words = total.div_ceil(64) + 1;
                let mut bits = vec![0u64; words];
                for (i, &b) in s.iter().enumerate() {
                    if b == b'\n' {
                        let at = i + delay as usize;
                        bits[at / 64] |= 1 << (at % 64);
                    }
                }
                // Bits outside the source are not its newlines.
                bits[words - 1] |= 1 << 63;
                if delay > 0 {
                    bits[0] |= 1;
                }
                let (m, offsets) = bits_of(s.len() as u64, delay, words, |w| bits[w]);
                let mut got = vec![0xa5; 3];
                index_of(&mut got, s.len() as u64, m, offsets);
                assert_eq!(got, want, "{} bytes, delay {delay}", s.len());
            }
        }
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
