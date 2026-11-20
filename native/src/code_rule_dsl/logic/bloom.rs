//! A blocked Bloom filter of a column's keys (decision 0477): the probe screen's (`native::setfix::join_filter`) and
//! the statistics' (a column's membership and the keys two columns share).
//!
//! ```text
//! h     = hash(key)                       // setfix's sketch hash: the same keys, the same h, everywhere
//! word  = (h >> 40) & (words - 1)         // one 64-bit word holds all of a key's bits
//! mask  = bit(h) | bit(h >> 6) | ... (5)  // 5 bits of the word, from 6-bit fields of h
//! add:  words[word] |= mask               // a device builds it with 32-bit atomics: bit b of word w is bit b & 31
//! test: words[word] & mask == mask        //   of the 32-bit word 2w + b / 32, the same bytes
//! ```
//!
//! A test is one hash, one load and one compare. A miss is certain; a hit is a candidate an exact probe checks.
//! Two filters of one size estimate the keys they share: each one's keys from its bits, less their union's.

/// Bits of a word a key sets.
pub const K: u32 = 5;
/// The bits a filter is sized to give each distinct key.
pub const BITS_PER_KEY: usize = 16;
/// The fewest and most words a filter has (256 B to 16 KiB).
pub const MIN_WORDS: usize = 32;
pub const MAX_WORDS: usize = 2048;

/// Setfix's sketch hash (`native::setfix::sketch::mix`), which the HyperLogLog sketches hash by too.
#[inline]
pub fn hash(v: i64) -> u64 {
    let mut x = v as u64 ^ 0x9e37_79b9_7f4a_7c15;
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// A key's word (of `words`, a power of two) and its bits there.
#[inline]
pub fn place(h: u64, words: usize) -> (usize, u64) {
    let mask = (0..K).fold(0u64, |m, i| m | 1 << ((h >> (6 * i)) & 63));
    ((h >> 40) as usize & (words - 1), mask)
}

/// The words for `keys` distinct keys: `BITS_PER_KEY` each, a power of two within the bounds.
pub fn words_for(keys: usize) -> usize {
    (keys.saturating_mul(BITS_PER_KEY) / 64).next_power_of_two().clamp(MIN_WORDS, MAX_WORDS)
}

/// A filter's words. Its bytes are the device's, as built (`place`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filter {
    pub words: Box<[u64]>,
}

impl Filter {
    /// An empty filter of `words` words (a power of two).
    pub fn new(words: usize) -> Filter {
        assert!(words.is_power_of_two(), "a filter's words are a power of two");
        Filter { words: vec![0; words].into_boxed_slice() }
    }

    #[inline]
    pub fn add(&mut self, key: i64) {
        let (w, m) = place(hash(key), self.words.len());
        self.words[w] |= m;
    }

    /// Whether `key` may be in: false is certain.
    #[inline]
    pub fn may_hold(&self, key: i64) -> bool {
        let (w, m) = place(hash(key), self.words.len());
        self.words[w] & m == m
    }

    /// The bits set.
    pub fn ones(&self) -> u64 {
        self.words.iter().map(|w| u64::from(w.count_ones())).sum()
    }

    /// The distinct keys added, estimated from the bits set (Swamidass and Baldi): `-(m / k) ln(1 - x / m)`.
    pub fn keys(&self) -> f64 {
        keys_of(self.ones(), self.words.len())
    }

    /// The distinct keys both filters hold, estimated (the two the same size, else `None`): each one's keys less
    /// their union's. Rough where the shared keys are few against the union.
    pub fn shared(&self, other: &Filter) -> Option<f64> {
        (self.words.len() == other.words.len()).then(|| {
            let union: u64 = self.words.iter().zip(&*other.words).map(|(a, b)| u64::from((a | b).count_ones())).sum();
            (self.keys() + other.keys() - keys_of(union, self.words.len())).clamp(0.0, self.keys().min(other.keys()))
        })
    }
}

fn keys_of(ones: u64, words: usize) -> f64 {
    let m = (words * 64) as f64;
    let x = (ones as f64).min(m - 1.0);
    -(m / f64::from(K)) * (1.0 - x / m).ln()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every key added is held; keys not added are mostly refused, near the rate the size gives.
    #[test]
    fn a_filter_holds_its_keys_and_refuses_most_others() {
        let mut f = Filter::new(words_for(1659));
        for k in 0..1659 {
            f.add(k * 7919);
        }
        assert!((0..1659).all(|k| f.may_hold(k * 7919)));
        let passed = (0..100_000).filter(|&k| f.may_hold(-1 - k)).count();
        assert!(passed < 3_000, "{passed} of 100000 passed");
        let keys = f.keys();
        assert!((keys / 1659.0 - 1.0).abs() < 0.1, "{keys}");
    }

    /// Two filters' shared keys: 626 of 7,650 and 7,650 (the SLF001 capture's shape), within a fifth.
    #[test]
    fn two_filters_estimate_the_keys_they_share() {
        let words = words_for(8000);
        let (mut a, mut b) = (Filter::new(words), Filter::new(words));
        for k in 0..7650 {
            a.add(k);
            b.add(k + 7650 - 626);
        }
        let shared = a.shared(&b).unwrap();
        assert!((shared / 626.0 - 1.0).abs() < 0.2, "{shared}");
        assert!(Filter::new(words).shared(&Filter::new(words * 2)).is_none());
    }
}
