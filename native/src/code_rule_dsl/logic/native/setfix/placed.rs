//! A relation's dedup test by row hash: its row ids placed in a table by their rows' hashes, for a relation no probe
//! reads by a key (most kept rows: O125, 85–91% of them), in place of the order on every column.
//!
//! The order on every column answers the dedup test by a sort of each wave's rows and a search of its runs; nothing
//! else reads it in such a relation. Placed by hash (as mldb's `IdHash`), the test is a lookup a row, and nothing is
//! sorted or merged:
//!
//! ```text
//! slot     u32: an 8-bit fingerprint of the row's hash over a 24-bit row id; EMPTY is u32::MAX
//! home     (hash × slots) >> 64: the hash's high bits, so a table of any size (not a power of two) is placed by it
//! probing  linear from home, to the first EMPTY: a slot whose fingerprint differs is passed without its row read
//! load     0.6 to 0.8: grown by 4/3 past 0.8, each id placed again by its row's hash (each row hashed about four
//!          times over the table's growth); 4 bytes a slot is 5 to 6.7 bytes a row
//! ```
//!
//! Dead rows stay placed (as they stay in an order's runs); the reader's `eq` skips them. Ids past 2^24 − 2 do not
//! fit: such a relation is ordered instead (`fits`).

/// No id: every bit set (fingerprint 0xff over id 0xff_ffff, an id never placed).
const EMPTY: u32 = u32::MAX;
const ID_BITS: u32 = 24;
const ID_MASK: u32 = (1 << ID_BITS) - 1;
/// The most rows a table places.
pub(super) const MAX_ROWS: usize = ID_MASK as usize;

#[derive(Clone, Default)]
pub(super) struct Placed {
    slots: Vec<u32>,
    len: usize,
}

/// The row hash mixed again (splitmix64's finisher): `hash_vals`'s FxHash leaves the low bits weak.
#[inline]
fn mix(h: u64) -> u64 {
    let mut x = h ^ (h >> 30);
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

#[inline]
fn home(h: u64, slots: usize) -> usize {
    ((u128::from(h) * slots as u128) >> 64) as usize
}

#[inline]
fn fingerprint(h: u64) -> u32 {
    (h as u32 & 0xff) << ID_BITS
}

impl Placed {
    /// Whether a relation of `rows` rows can be placed.
    pub(super) fn fits(rows: usize) -> bool {
        rows <= MAX_ROWS
    }

    /// Ids placed.
    pub(super) fn len(&self) -> usize {
        self.len
    }

    /// The placed id whose row has hash `h` (`hash_vals`) and passes `eq` (equal, and live).
    #[inline]
    pub(super) fn find(&self, h: u64, eq: impl Fn(u32) -> bool) -> Option<u32> {
        let n = self.slots.len();
        if n == 0 {
            return None;
        }
        let h = mix(h);
        let fp = fingerprint(h);
        let mut i = home(h, n);
        loop {
            let s = self.slots[i];
            if s == EMPTY {
                return None;
            }
            if s & !ID_MASK == fp && eq(s & ID_MASK) {
                return Some(s & ID_MASK);
            }
            i += 1;
            if i == n {
                i = 0;
            }
        }
    }

    /// Places `id`, whose row has hash `h`; `hash_of` gives a placed id's row hash, for the table's growth. The
    /// caller places each row once (its dedup test found none equal).
    pub(super) fn insert(&mut self, h: u64, id: u32, hash_of: impl Fn(u32) -> u64) {
        assert!((id as usize) < MAX_ROWS, "a placed table holds ids below 2^24");
        if (self.len + 1) * 5 > self.slots.len() * 4 {
            self.grow(self.len + 1, &hash_of);
        }
        Self::place(&mut self.slots, mix(h), id);
        self.len += 1;
    }

    /// Room for `more` ids past those placed, grown once rather than as they come.
    pub(super) fn reserve(&mut self, more: usize, hash_of: impl Fn(u32) -> u64) {
        if (self.len + more) * 5 > self.slots.len() * 4 {
            self.grow(self.len + more, &hash_of);
        }
    }

    fn grow(&mut self, want: usize, hash_of: &impl Fn(u32) -> u64) {
        // At 0.6 of the new size: a third more slots than the load's top needs.
        let n = (want * 5 / 3).max(self.slots.len() * 4 / 3).max(16);
        let mut slots = vec![EMPTY; n];
        for &s in &self.slots {
            if s != EMPTY {
                Self::place(&mut slots, mix(hash_of(s & ID_MASK)), s & ID_MASK);
            }
        }
        self.slots = slots;
    }

    #[inline]
    fn place(slots: &mut [u32], h: u64, id: u32) {
        let n = slots.len();
        let mut i = home(h, n);
        while slots[i] != EMPTY {
            i += 1;
            if i == n {
                i = 0;
            }
        }
        slots[i] = fingerprint(h) | id;
    }

    /// The placed ids, in no order.
    pub(super) fn ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.slots.iter().filter(|&&s| s != EMPTY).map(|&s| s & ID_MASK)
    }

    pub(super) fn bytes(&self) -> usize {
        self.slots.capacity() * 4
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    fn placed_ids_are_found_by_their_rows_hash() {
        let k = 2;
        let rows: Vec<i64> = (0..10_000i64).flat_map(|i| [i % 97, i]).collect();
        let hash_of = |id: u32| super::super::hash_vals(rows[id as usize * k..][..k].iter().copied());
        let mut p = Placed::default();
        for id in 0..10_000u32 {
            p.insert(hash_of(id), id, hash_of);
        }
        assert_eq!(p.len(), 10_000);
        // Within the load's bounds: 5 to 6.7 bytes a row.
        let per = p.bytes() as f64 / 10_000.0;
        assert!((4.9..=6.8).contains(&per), "{per} bytes a row");
        for id in (0..10_000u32).step_by(7) {
            let row = &rows[id as usize * k..][..k];
            let eq = |x: u32| &rows[x as usize * k..][..k] == row;
            assert_eq!(p.find(hash_of(id), eq), Some(id));
        }
        let absent = [5i64, 5];
        let h = super::super::hash_vals(absent.iter().copied());
        assert_eq!(p.find(h, |x| rows[x as usize * k..][..k] == absent), None);
        let mut ids: Vec<u32> = p.ids().collect();
        ids.sort_unstable();
        assert!(ids.iter().copied().eq(0..10_000));
    }
}
