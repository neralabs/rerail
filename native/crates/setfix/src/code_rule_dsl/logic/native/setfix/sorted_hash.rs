//! A sorted hash (t50 step 5; the user's choice of 2026-11-23, measured): an index of a relation's rows by a key of
//! its columns, the row ids in the order of a 64-bit hash of the key, the hashes beside them. A key is placed by its
//! hash's high bits and galloped to its first equal hash: a probe is a few loads, where an index's runs search a
//! 64-id block and read each row through its id (100–285 ns a probe on `walk`'s and `look`'s tests, against 29–77 ns
//! here: `runs/index_spike.rs`). It holds 12 bytes a row, no value of a row: the rows are read where they lie, each
//! candidate's key checked against them (equal hashes are candidates, not answers).

/// The ids of a relation's rows in the order of their key's hash (`hash_key`), the hashes beside them.
pub(super) struct SortedHash {
    hashes: Box<[u64]>,
    ids: Box<[u32]>,
}

#[inline]
fn mix(mut h: u64) -> u64 {
    // splitmix64's finalizer: every bit of the key reaches the high bits a probe is placed by.
    h ^= h >> 30;
    h = h.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^ (h >> 31)
}

/// A key's hash: its values in the index's column order.
#[inline]
pub(super) fn hash_key(key: impl IntoIterator<Item = i64>) -> u64 {
    let mut h = 0x9E37_79B9_7F4A_7C15u64;
    for v in key {
        h = (h.rotate_left(23) ^ v as u64).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
    mix(h)
}

impl SortedHash {
    /// Of `n` rows, row `id`'s key hashed by `key_of`.
    pub(super) fn new(n: usize, key_of: impl Fn(u32) -> u64) -> SortedHash {
        let mut v: Vec<(u64, u32)> = (0..n as u32).map(|id| (key_of(id), id)).collect();
        v.sort_unstable();
        SortedHash { hashes: v.iter().map(|p| p.0).collect(), ids: v.iter().map(|p| p.1).collect() }
    }

    /// The ids whose key hashes to `h`: candidates, each to be checked against its row.
    #[inline]
    pub(super) fn candidates(&self, h: u64) -> &[u32] {
        let n = self.hashes.len();
        if n == 0 {
            return &[];
        }
        let guess = ((u128::from(h) * n as u128) >> 64) as usize;
        let s = super::runs::gallop(&self.hashes, guess, |&x| x < h);
        let e = s + self.hashes[s..].iter().take_while(|&&x| x == h).count();
        &self.ids[s..e]
    }

    /// Its bytes.
    pub(super) fn bytes(&self) -> usize {
        self.hashes.len() * 12
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every key's rows are among its candidates, and a key no row holds has none of its own.
    #[test]
    fn a_sorted_hash_finds_every_keys_rows() {
        let rows: Vec<[i64; 2]> = (0..5000i64).map(|i| [(i * 7919) % 613, i % 37]).collect();
        let ix = SortedHash::new(rows.len(), |id| hash_key(rows[id as usize]));
        for (id, r) in rows.iter().enumerate() {
            let found: Vec<u32> = ix.candidates(hash_key(*r)).iter().copied().filter(|&c| rows[c as usize] == *r).collect();
            let want: Vec<u32> = rows.iter().enumerate().filter(|(_, q)| *q == r).map(|(i, _)| i as u32).collect();
            let mut found = found;
            found.sort_unstable();
            assert_eq!(found, want, "row {id}'s key");
        }
        assert!(ix.candidates(hash_key([-1, -1])).iter().all(|&c| rows[c as usize] != [-1, -1]));
        assert_eq!(ix.bytes(), 12 * rows.len());
    }
}
