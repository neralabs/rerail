//! Spike (the index cost, 2026-11-10): a relation's indexes as column orders, each a sorted run of row ids.
//!
//! Today every kept relation has a hash index on each of its columns: a chain slot a row and a posting a key, per
//! column, beside the dedup set (about 63 bytes a row at O125). Its probes bind few key-column sets, and those are
//! mostly prefixes of each other (`pv` is probed on [0], [0, 1], [0, 1, 2], [1] and [1, 2]). An index that sorts the
//! row ids by the values of its columns in an order answers a probe on any prefix of the order, and one whose order
//! takes every column answers the dedup test too. So a relation needs one order per chain of its probed key sets
//! (a chain cover), each 4 bytes a row, plus a fence a block (the order's leading value every `FENCE` ids, 8 bytes),
//! searched in cache before the block's ids are.
//!
//! Only the spike's measurement builds these (`InRel::index_spike`, `RERAIL_INDEX_SPIKE`): nothing reads them.

use std::cmp::Ordering;

/// Ids a fence covers.
pub(super) const FENCE: usize = 64;

/// The row ids of a relation's live rows, sorted by their values in `order` (then by id), with the leading value of
/// every `FENCE`th id.
pub(super) struct Ordered {
    pub(super) order: Vec<usize>,
    ids: Vec<u32>,
    fences: Vec<i64>,
    /// Sorted by the leading value's hash first (`mix`), and searched by interpolation on it: no fences.
    hashed: bool,
}

/// A value's 64-bit hash, uniform enough to interpolate on.
#[inline]
pub(super) fn mix(v: i64) -> u64 {
    let mut x = v as u64 ^ 0x9e37_79b9_7f4a_7c15;
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

impl Ordered {
    /// Sorts the live rows of `rows` (`k` values each, `n` rows; `dead` says which are not) by `order`.
    pub(super) fn build(rows: &[i64], k: usize, n: usize, dead: impl Fn(u32) -> bool, order: Vec<usize>, hashed: bool) -> Ordered {
        let mut ids: Vec<u32> = (0..n as u32).filter(|&i| !dead(i)).collect();
        let row = |id: u32| &rows[id as usize * k..][..k];
        let lead = order[0];
        if hashed {
            ids.sort_by_cached_key(|&id| mix(row(id)[lead]));
            // Then by the values, within each run of one hash (a stable sort keeps id order last).
            let mut at = 0;
            while at < ids.len() {
                let h = mix(row(ids[at])[lead]);
                let end = at + ids[at..].partition_point(|&id| mix(row(id)[lead]) == h);
                ids[at..end].sort_by(|&a, &b| {
                    let (ra, rb) = (row(a), row(b));
                    order.iter().map(|&c| ra[c].cmp(&rb[c])).find(|o| o.is_ne()).unwrap_or_else(|| a.cmp(&b))
                });
                at = end;
            }
            return Ordered { order, ids, fences: Vec::new(), hashed };
        }
        ids.sort_unstable_by(|&a, &b| {
            let (ra, rb) = (row(a), row(b));
            order.iter().map(|&c| ra[c].cmp(&rb[c])).find(|o| o.is_ne()).unwrap_or_else(|| a.cmp(&b))
        });
        let fences = ids.iter().step_by(FENCE).map(|&id| row(id)[order[0]]).collect();
        Ordered { order, ids, fences, hashed }
    }

    /// The ids of the rows whose first `key.len()` columns of the order hold `key`.
    pub(super) fn range(&self, rows: &[i64], k: usize, key: &[i64]) -> &[u32] {
        if self.ids.is_empty() || key.is_empty() {
            return &self.ids;
        }
        let lead = key[0];
        if self.hashed {
            return self.range_hashed(rows, k, key);
        }
        // The blocks that may hold the leading value: from the last fence below it to the first fence above it.
        let lo = self.fences.partition_point(|&f| f < lead).saturating_sub(1) * FENCE;
        let hi = (self.fences.partition_point(|&f| f <= lead) * FENCE).min(self.ids.len());
        let block = &self.ids[lo..hi];
        let cmp = |id: u32| -> Ordering {
            let r = &rows[id as usize * k..][..k];
            self.order.iter().zip(key).map(|(&c, v)| r[c].cmp(v)).find(|o| o.is_ne()).unwrap_or(Ordering::Equal)
        };
        let start = block.partition_point(|&id| cmp(id) == Ordering::Less);
        let end = start + block[start..].partition_point(|&id| cmp(id) == Ordering::Equal);
        &block[start..end]
    }

    /// `range` over ids sorted by the leading value's hash: the position the hash interpolates to, galloped out to
    /// bracket the hash's run, then that run searched by the key.
    fn range_hashed(&self, rows: &[i64], k: usize, key: &[i64]) -> &[u32] {
        let (ids, lead) = (&self.ids, self.order[0]);
        let h = mix(key[0]);
        let hash_at = |i: usize| mix(rows[ids[i] as usize * k + lead]);
        let n = ids.len();
        let guess = ((h as u128 * n as u128) >> 64) as usize;
        // Bracket [lo, hi) with hash_at(lo - 1) < h <= hash_at(hi) ... by galloping from the guess.
        let (mut lo, mut hi);
        if hash_at(guess) < h {
            let mut step = 1;
            lo = guess + 1;
            hi = (guess + step).min(n);
            while hi < n && hash_at(hi) < h {
                lo = hi + 1;
                step *= 2;
                hi = (guess + step).min(n);
            }
            hi = (hi + 1).min(n);
        } else {
            let mut step = 1;
            hi = guess + 1;
            lo = guess.saturating_sub(step);
            while lo > 0 && hash_at(lo) >= h {
                hi = lo + 1;
                step *= 2;
                lo = guess.saturating_sub(step);
            }
        }
        // The run of the hash, then the key within it.
        let first = lo + ids[lo..hi].partition_point(|&id| mix(rows[id as usize * k + lead]) < h);
        let mut end = first;
        while end < n && hash_at(end) == h {
            end += 1;
        }
        let run = &ids[first..end];
        let cmp = |id: u32| -> Ordering {
            let r = &rows[id as usize * k..][..k];
            self.order.iter().zip(key).map(|(&c, v)| r[c].cmp(v)).find(|o| o.is_ne()).unwrap_or(Ordering::Equal)
        };
        let start = run.partition_point(|&id| cmp(id) == Ordering::Less);
        let stop = start + run[start..].partition_point(|&id| cmp(id) == Ordering::Equal);
        &run[start..stop]
    }

    /// The bytes it holds.
    pub(super) fn bytes(&self) -> usize {
        self.ids.capacity() * 4 + self.fences.capacity() * 8
    }

    /// The length of the prefix of the order that is exactly `cols` (as a set), if one is.
    pub(super) fn prefix(&self, cols: &[usize]) -> Option<usize> {
        let n = cols.len();
        (n <= self.order.len() && self.order[..n].iter().all(|c| cols.contains(c))).then_some(n)
    }
}

/// A key's hash: one multiply a column (the Fx step), then one avalanche so the top bits, which place it, are
/// uniform.
#[inline]
pub(super) fn key_hash(vals: impl Iterator<Item = i64>) -> u64 {
    let mut h = vals.fold(0u64, |h, v| (h.rotate_left(5) ^ v as u64).wrapping_mul(0x517c_c1b7_2722_0a95));
    h ^= h >> 29;
    h = h.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h ^ (h >> 32)
}

/// mldb's frozen `PathIndex` (the IdHash idea, `plugins/tabular/tabular_dataset.cc`): the live rows sorted by the hash
/// of their key columns and each placed at the slot its hash interpolates to (`hash * m / 2^64`), or the first free
/// slot after the one before it. Slot order is hash order, so a key's rows are in one run that starts at or after its
/// home slot and ends before the next empty slot. A slot is 32 bits: the row id + 1, and above it as many bits of the
/// hash as fit (a fingerprint that skips most rows without reading them).
pub(super) struct Placed {
    pub(super) cols: Vec<usize>,
    slots: Vec<u32>,
    m: usize,
    id_bits: u32,
}

impl Placed {
    pub(super) fn build(rows: &[i64], k: usize, n: usize, dead: impl Fn(u32) -> bool, cols: Vec<usize>, load: f64) -> Placed {
        let row = |id: u32| &rows[id as usize * k..][..k];
        let mut hs: Vec<(u64, u32)> = (0..n as u32).filter(|&i| !dead(i)).map(|id| (key_hash(cols.iter().map(|&c| row(id)[c])), id)).collect();
        hs.sort_unstable();
        let m = ((hs.len() as f64 / load).ceil() as usize).max(1);
        let id_bits = 32 - (n as u32 + 1).leading_zeros();
        let mut slots = vec![0u32; m + 1];
        let mut next = 0usize;
        for &(h, id) in &hs {
            let at = (((h as u128) * (m as u128)) >> 64) as usize;
            let at = at.max(next);
            if at + 1 >= slots.len() {
                slots.resize(at + 2, 0);
            }
            slots[at] = Self::enc(id, h, id_bits);
            next = at + 1;
        }
        slots.shrink_to_fit();
        Placed { cols, slots, m, id_bits }
    }

    #[inline]
    fn fp(h: u64, id_bits: u32) -> u32 {
        if id_bits >= 32 { 0 } else { (h as u32) >> id_bits }
    }

    #[inline]
    fn enc(id: u32, h: u64, id_bits: u32) -> u32 {
        if id_bits >= 32 { id + 1 } else { (Self::fp(h, id_bits) << id_bits) | (id + 1) }
    }

    /// Calls `f` on each live row id whose key columns hold `key` (in `cols` order).
    #[inline]
    pub(super) fn each(&self, rows: &[i64], k: usize, key: &[i64], mut f: impl FnMut(u32)) {
        let h = key_hash(key.iter().copied());
        let (ib, fp) = (self.id_bits, Self::fp(h, self.id_bits));
        let mask = if ib >= 32 { u32::MAX } else { (1u32 << ib) - 1 };
        let mut at = (((h as u128) * (self.m as u128)) >> 64) as usize;
        while let Some(&s) = self.slots.get(at) {
            if s == 0 {
                break;
            }
            if ib >= 32 || s >> ib == fp {
                let id = (s & mask) - 1;
                let r = &rows[id as usize * k..][..k];
                if self.cols.iter().zip(key).all(|(&c, v)| r[c] == *v) {
                    f(id);
                }
            }
            at += 1;
        }
    }

    pub(super) fn bytes(&self) -> usize {
        self.slots.capacity() * 4
    }
}

/// Column orders that cover `sets` (key-column sets, each sorted) and the whole row of `k` columns: each set is a
/// prefix of one order, with as few orders as can be (a minimum chain cover of the sets by inclusion: a maximum
/// matching of each set to a larger one holding it, Dilworth). Each order is its chain's sets' columns in turn, then
/// the rest.
pub(super) fn cover(sets: &[Vec<usize>], k: usize) -> Vec<Vec<usize>> {
    let mut sets: Vec<Vec<usize>> = sets.iter().filter(|s| !s.is_empty()).cloned().collect();
    sets.push((0..k).collect());
    sets.sort_by_key(|s| (s.len(), s.clone()));
    sets.dedup();
    let n = sets.len();
    let below = |a: usize, b: usize| sets[a].len() < sets[b].len() && sets[a].iter().all(|x| sets[b].contains(x));
    // next[a] = b: a's chain goes on to b; prev[b] = a.
    let (mut next, mut prev) = (vec![usize::MAX; n], vec![usize::MAX; n]);
    fn augment(a: usize, seen: &mut [bool], next: &mut [usize], prev: &mut [usize], below: &dyn Fn(usize, usize) -> bool, n: usize) -> bool {
        for b in 0..n {
            if below(a, b) && !seen[b] {
                seen[b] = true;
                if prev[b] == usize::MAX || augment(prev[b], seen, next, prev, below, n) {
                    next[a] = b;
                    prev[b] = a;
                    return true;
                }
            }
        }
        false
    }
    for a in 0..n {
        let mut seen = vec![false; n];
        augment(a, &mut seen, &mut next, &mut prev, &below, n);
    }
    (0..n)
        .filter(|&a| prev[a] == usize::MAX)
        .map(|start| {
            let mut order: Vec<usize> = Vec::new();
            let mut at = start;
            loop {
                let add: Vec<usize> = sets[at].iter().filter(|c| !order.contains(c)).copied().collect();
                order.extend(add);
                if next[at] == usize::MAX {
                    break;
                }
                at = next[at];
            }
            let rest: Vec<usize> = (0..k).filter(|c| !order.contains(c)).collect();
            order.extend(rest);
            order
        })
        .collect()
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    fn a_cover_puts_prefix_sets_in_one_order_and_ends_each_with_every_column() {
        let orders = cover(&[vec![0], vec![0, 1], vec![0, 1, 2], vec![1], vec![1, 2]], 3);
        assert_eq!(orders.len(), 2, "{orders:?}");
        assert!(orders.iter().all(|o| o.len() == 3));
        for s in [vec![0], vec![0, 1], vec![1], vec![1, 2], vec![0, 1, 2]] {
            assert!(orders.iter().any(|o| o[..s.len()].iter().all(|c| s.contains(c))), "{s:?} in {orders:?}");
        }
    }

    #[test]
    fn a_range_is_the_rows_holding_the_key_prefix() {
        // 3 columns, 1000 rows; column 0 has 10 values, column 1 has 7.
        let k = 3;
        let rows: Vec<i64> = (0..1000i64).flat_map(|i| [i % 10, i % 7, i]).collect();
        for hashed in [false, true] {
            let ix = Ordered::build(&rows, k, 1000, |id| id == 5, vec![0, 1, 2], hashed);
            for (key, want) in [(vec![3i64], 100usize), (vec![3, 4], 14), (vec![5], 99), (vec![11], 0), (vec![-4], 0)] {
                let got = ix.range(&rows, k, &key);
                assert_eq!(got.len(), want, "{key:?} hashed {hashed}");
                assert!(got.iter().all(|&id| key.iter().enumerate().all(|(i, v)| rows[id as usize * k + i] == *v)));
            }
            assert_eq!(ix.range(&rows, k, &[3, 3, 3]).len(), 1);
            assert_eq!(ix.range(&rows, k, &[3, 3, 4]).len(), 0);
        }
    }

    #[test]
    fn a_placed_index_finds_every_row_of_a_key_and_no_other() {
        let k = 3;
        let rows: Vec<i64> = (0..1000i64).flat_map(|i| [i % 10, i % 7, i]).collect();
        for load in [0.5, 0.75, 0.95] {
            let ix = Placed::build(&rows, k, 1000, |id| id == 5, vec![0, 1], load);
            for (key, want) in [(vec![3i64, 4], 14usize), (vec![5, 5], 14), (vec![11, 0], 0)] {
                let mut got = 0;
                ix.each(&rows, k, &key, |id| { assert_eq!(&rows[id as usize * k..][..2], &key[..]); got += 1 });
                assert_eq!(got, want, "{key:?} load {load}");
            }
            let full = Placed::build(&rows, k, 1000, |id| id == 5, vec![0, 1, 2], load);
            let mut got = 0;
            full.each(&rows, k, &[5, 5, 5], |_| got += 1);
            assert_eq!(got, 0);
            full.each(&rows, k, &[6, 6, 6], |_| got += 1);
            assert_eq!(got, 1);
        }
    }
}

/// A placed index that grows (the incremental form of `Placed`, mldb's IdHash growth): a row goes in at the first
/// empty slot from its home (the top `b` bits of its hash, `m = 2^b` homes), never wrapping (the slots run on past
/// `m` as needed), so a key's rows are between its home and the next empty slot. A slot is 32 bits: the row id + 1,
/// its distance from its home (`DISP` bits, saturated), and the hash bits just below the home bits (the
/// fingerprint). At three quarters full `m` doubles, and each row's new home is twice its old one (its slot less its
/// distance) plus its fingerprint's top bit, so no row is read; the fingerprint shifts up and loses a bit. Once fewer
/// than `MIN_FP` of its bits are known, or for a saturated distance, the doubling reads the row and hashes it again.
pub(super) struct Growing {
    pub(super) cols: Vec<usize>,
    slots: Vec<u32>,
    b: u32,
    len: usize,
    id_bits: u32,
    fp_bits: u32,
    /// The known (top) bits of every slot's fingerprint.
    fp_known: u32,
    pub(super) grew: usize,
    /// Rows read again to place them (saturated distances, or a doubling with too few fingerprint bits known).
    pub(super) rehashed: usize,
}

const DISP: u32 = 4;
const MAX_DISP: usize = (1 << DISP) - 1;
const MIN_FP: u32 = 2;

impl Growing {
    pub(super) fn new(cols: Vec<usize>, id_bits: u32) -> Growing {
        let fp_bits = 32u32.saturating_sub(id_bits + DISP);
        Growing { cols, slots: vec![0; 64], b: 6, len: 0, id_bits, fp_bits, fp_known: fp_bits, grew: 0, rehashed: 0 }
    }

    #[inline]
    fn home(&self, h: u64) -> usize {
        (h >> (64 - self.b)) as usize
    }

    /// The `fp_bits` bits of `h` below its top `b`.
    #[inline]
    fn fp_of(&self, h: u64, b: u32) -> u32 {
        if self.fp_bits == 0 { 0 } else { ((h << b) >> (64 - self.fp_bits)) as u32 }
    }

    #[inline]
    fn enc(&self, id: u32, disp: usize, fp: u32) -> u32 {
        let low = (disp.min(MAX_DISP) as u32) << self.id_bits | (id + 1);
        if self.fp_bits == 0 { low } else { fp << (self.id_bits + DISP) | low }
    }

    #[inline]
    fn dec(&self, s: u32) -> (u32, usize, u32) {
        let id = (s & ((1u32 << self.id_bits) - 1)) - 1;
        let disp = ((s >> self.id_bits) & MAX_DISP as u32) as usize;
        let fp = if self.fp_bits == 0 { 0 } else { s >> (self.id_bits + DISP) };
        (id, disp, fp)
    }

    #[inline]
    fn fp_matches(&self, a: u32, b: u32) -> bool {
        self.fp_known == 0 || (a ^ b) >> (self.fp_bits - self.fp_known) == 0
    }

    /// Places `s`-like content for row `id` at the first empty slot from `home` in `slots`.
    #[inline]
    fn place(slots: &mut Vec<u32>, home: usize) -> usize {
        let mut at = home;
        while at < slots.len() && slots[at] != 0 {
            at += 1;
        }
        if at >= slots.len() {
            slots.resize(at + 1, 0);
        }
        at
    }

    /// Room for `more` rows at load 0.75 or less.
    pub(super) fn reserve(&mut self, rows: &[i64], k: usize, more: usize) {
        while (self.len + more) * 4 > (1usize << self.b) * 3 {
            self.double(rows, k);
        }
    }

    fn double(&mut self, rows: &[i64], k: usize) {
        let b = self.b + 1;
        let read_all = self.fp_known < MIN_FP;
        let old = std::mem::replace(&mut self.slots, vec![0; (1usize << b) + 1]);
        self.grew += 1;
        for (at, &s) in old.iter().enumerate() {
            if s == 0 {
                continue;
            }
            let (id, disp, fp) = self.dec(s);
            let (home, fp) = if read_all || disp == MAX_DISP || self.fp_bits == 0 {
                self.rehashed += 1;
                let h = key_hash(self.cols.iter().map(|&c| rows[id as usize * k + c]));
                ((h >> (64 - b)) as usize, self.fp_of(h, b))
            } else {
                // The old home and the next hash bit (the fingerprint's top bit).
                let old_home = at - disp;
                (old_home * 2 + (fp >> (self.fp_bits - 1)) as usize, (fp << 1) & ((1u32 << self.fp_bits) - 1))
            };
            let to = Self::place(&mut self.slots, home);
            self.slots[to] = self.enc(id, to - home, fp);
        }
        self.b = b;
        self.fp_known = if read_all { self.fp_bits } else { self.fp_known.saturating_sub(1) };
    }

    /// Places row `id` (hash `h`); with `dedup`, not when a row equal on `cols` is there (returns false).
    #[inline]
    pub(super) fn insert(&mut self, rows: &[i64], k: usize, id: u32, h: u64, dedup: bool) -> bool {
        let home = self.home(h);
        let fp = self.fp_of(h, self.b);
        let mut at = home;
        loop {
            if at >= self.slots.len() {
                self.slots.resize(at + 1, 0);
            }
            let s = self.slots[at];
            if s == 0 {
                self.slots[at] = self.enc(id, at - home, fp);
                self.len += 1;
                return true;
            }
            if dedup {
                let (x, _, f) = self.dec(s);
                if self.fp_matches(f, fp) && self.cols.iter().all(|&c| rows[x as usize * k + c] == rows[id as usize * k + c]) {
                    return false;
                }
            }
            at += 1;
        }
    }

    /// Whether a row equal to row `id` on `cols` is held (the dedup test of a duplicate offer).
    #[inline]
    pub(super) fn insert_probe(&self, rows: &[i64], k: usize, id: u32, h: u64) -> bool {
        let fp = self.fp_of(h, self.b);
        let mut at = self.home(h);
        while let Some(&s) = self.slots.get(at) {
            if s == 0 {
                return false;
            }
            let (x, _, f) = self.dec(s);
            if self.fp_matches(f, fp) && self.cols.iter().all(|&c| rows[x as usize * k + c] == rows[id as usize * k + c]) {
                return true;
            }
            at += 1;
        }
        false
    }

    /// Calls `f` on each row id whose key columns hold `key` (in `cols` order).
    pub(super) fn each(&self, rows: &[i64], k: usize, key: &[i64], mut f: impl FnMut(u32)) {
        let h = key_hash(key.iter().copied());
        let fp = self.fp_of(h, self.b);
        let mut at = self.home(h);
        while let Some(&s) = self.slots.get(at) {
            if s == 0 {
                break;
            }
            let (x, _, sf) = self.dec(s);
            if self.fp_matches(sf, fp) && self.cols.iter().zip(key).all(|(&c, v)| rows[x as usize * k + c] == *v) {
                f(x);
            }
            at += 1;
        }
    }

    pub(super) fn bytes(&self) -> usize {
        self.slots.capacity() * 4
    }

    pub(super) fn clone_slots(&self) -> Vec<u32> {
        self.slots.clone()
    }
}

/// Sorted orders that grow by runs (size-tiered): each wave's new ids sorted by the order are a run, the newest of
/// which is the wave's delta. Before a wave adds its run, the newest `fanout` runs are merged in one pass while the
/// oldest of them is no more than `fanout` times the newest: O(log_fanout n) passes over each id, and up to
/// `fanout` runs of each size for a probe to search.
pub(super) struct Runs {
    pub(super) order: Vec<usize>,
    pub(super) runs: Vec<Vec<u32>>,
    pub(super) merged: usize,
    fanout: usize,
}

impl Runs {
    pub(super) fn new(order: Vec<usize>) -> Runs {
        Runs::with_fanout(order, 2)
    }

    pub(super) fn with_fanout(order: Vec<usize>, fanout: usize) -> Runs {
        Runs { order, runs: Vec::new(), merged: 0, fanout: fanout.max(2) }
    }

    #[inline]
    pub(super) fn cmp(order: &[usize], rows: &[i64], k: usize, a: u32, b: u32) -> Ordering {
        let (ra, rb) = (&rows[a as usize * k..][..k], &rows[b as usize * k..][..k]);
        order.iter().map(|&c| ra[c].cmp(&rb[c])).find(|o| o.is_ne()).unwrap_or_else(|| a.cmp(&b))
    }

    /// Merges `parts` (each sorted) in one pass: each output id is the least of the parts' heads.
    fn merge(order: &[usize], rows: &[i64], k: usize, parts: Vec<Vec<u32>>) -> Vec<u32> {
        let total = parts.iter().map(Vec::len).sum();
        let mut out = Vec::with_capacity(total);
        let mut at = vec![0usize; parts.len()];
        if parts.len() == 2 {
            let (a, b) = (&parts[0], &parts[1]);
            let (mut i, mut j) = (0, 0);
            while i < a.len() && j < b.len() {
                if Self::cmp(order, rows, k, a[i], b[j]).is_le() { out.push(a[i]); i += 1 } else { out.push(b[j]); j += 1 }
            }
            out.extend_from_slice(&a[i..]);
            out.extend_from_slice(&b[j..]);
            return out;
        }
        loop {
            let mut best: Option<usize> = None;
            for (p, part) in parts.iter().enumerate() {
                if at[p] < part.len() && best.is_none_or(|q| Self::cmp(order, rows, k, part[at[p]], parts[q][at[q]]).is_lt()) {
                    best = Some(p);
                }
            }
            let Some(p) = best else { break };
            out.push(parts[p][at[p]]);
            at[p] += 1;
        }
        out
    }

    pub(super) fn add_wave(&mut self, rows: &[i64], k: usize, mut ids: Vec<u32>) {
        let t = self.fanout;
        while self.runs.len() >= t && self.runs[self.runs.len() - t].len() <= t * self.runs[self.runs.len() - 1].len() {
            let parts: Vec<Vec<u32>> = self.runs.split_off(self.runs.len() - t);
            self.merged += parts.iter().map(Vec::len).sum::<usize>();
            let out = Self::merge(&self.order, rows, k, parts);
            self.runs.push(out);
        }
        if ids.is_empty() {
            return;
        }
        let order = &self.order;
        ids.sort_unstable_by(|&a, &b| Self::cmp(order, rows, k, a, b));
        self.runs.push(ids);
    }

    /// Every run merged into one (the frozen order, to compare probes with).
    pub(super) fn merged_all(&self, rows: &[i64], k: usize) -> Runs {
        let one = Self::merge(&self.order, rows, k, self.runs.clone());
        Runs { order: self.order.clone(), runs: vec![one], merged: 0, fanout: self.fanout }
    }

    /// The number of ids holding `key` on the order's first `key.len()` columns, over every run.
    pub(super) fn count(&self, rows: &[i64], k: usize, key: &[i64]) -> usize {
        let cmp = |id: u32| -> Ordering {
            let r = &rows[id as usize * k..][..k];
            self.order.iter().zip(key).map(|(&c, v)| r[c].cmp(v)).find(|o| o.is_ne()).unwrap_or(Ordering::Equal)
        };
        self.runs.iter().map(|run| {
            let s = run.partition_point(|&id| cmp(id) == Ordering::Less);
            run[s..].partition_point(|&id| cmp(id) == Ordering::Equal)
        }).sum()
    }

    pub(super) fn bytes(&self) -> usize {
        self.runs.iter().map(|r| r.capacity() * 4).sum()
    }
}

/// A value's 16-bit tag: the top bits of its hash (`mix`).
#[inline]
pub(super) fn tag(v: i64) -> u16 {
    (mix(v) >> 48) as u16
}

/// The first index of `s` where `pred` fails, `pred` holding on a prefix: by doubling steps from the start.
fn gallop(s: &[u16], pred: impl Fn(u16) -> bool) -> usize {
    let mut e = 1;
    while e <= s.len() && pred(s[e - 1]) {
        e *= 2;
    }
    let lo = e / 2;
    let hi = e.min(s.len());
    lo + s[lo..hi].partition_point(|&x| pred(x))
}

/// The range of sorted `tags` equal to `t`, from its interpolated place.
fn interp_range(tags: &[u16], t: u16) -> (usize, usize) {
    let n = tags.len();
    if n == 0 {
        return (0, 0);
    }
    let g = ((t as usize * n) >> 16).min(n - 1);
    let lo = if tags[g] < t {
        g + 1 + gallop(&tags[g + 1..], |x| x < t)
    } else {
        // Leftwards from g: the first index whose tag is below t, by doubling steps.
        let mut e = 1;
        while e <= g && tags[g - e] >= t {
            e *= 2;
        }
        let start = g.saturating_sub(e);
        start + tags[start..=g].partition_point(|&x| x < t)
    };
    (lo, lo + gallop(&tags[lo..], |x| x == t))
}

/// `Runs` whose ids each carry their row's tag (the leading column's, `tag`), in a parallel array: 2 bytes an id. An
/// order sorts by the tag first, then by the values, so the rows of a leading value are still together. A merge
/// compares tags and reads two rows only when the tags are equal; a probe searches its tag in the tags (in cache),
/// then reads the rows of that tag only.
pub(super) struct TaggedRuns {
    pub(super) order: Vec<usize>,
    pub(super) runs: Vec<(Vec<u32>, Vec<u16>)>,
    pub(super) merged: usize,
    /// Rows read by merges' comparisons (the tags tied).
    pub(super) row_compares: usize,
    /// Of those, the ties of two different leading values: a collision of the tags (more tag bits would avoid it).
    pub(super) collisions: usize,
    fanout: usize,
}

impl TaggedRuns {
    pub(super) fn with_fanout(order: Vec<usize>, fanout: usize) -> TaggedRuns {
        TaggedRuns { order, runs: Vec::new(), merged: 0, row_compares: 0, collisions: 0, fanout: fanout.max(2) }
    }

    #[inline]
    fn rows_cmp(order: &[usize], rows: &[i64], k: usize, a: u32, b: u32) -> Ordering {
        Runs::cmp(order, rows, k, a, b)
    }

    fn merge2(&mut self, rows: &[i64], k: usize, a: (Vec<u32>, Vec<u16>), b: (Vec<u32>, Vec<u16>)) -> (Vec<u32>, Vec<u16>) {
        let n = a.0.len() + b.0.len();
        let (mut ids, mut tags) = (Vec::with_capacity(n), Vec::with_capacity(n));
        let (mut i, mut j) = (0, 0);
        while i < a.0.len() && j < b.0.len() {
            let le = match a.1[i].cmp(&b.1[j]) {
                Ordering::Less => true,
                Ordering::Greater => false,
                Ordering::Equal => {
                    self.row_compares += 1;
                    let lead = self.order[0];
                    self.collisions += usize::from(rows[a.0[i] as usize * k + lead] != rows[b.0[j] as usize * k + lead]);
                    Self::rows_cmp(&self.order, rows, k, a.0[i], b.0[j]).is_le()
                }
            };
            if le { ids.push(a.0[i]); tags.push(a.1[i]); i += 1 } else { ids.push(b.0[j]); tags.push(b.1[j]); j += 1 }
        }
        ids.extend_from_slice(&a.0[i..]);
        tags.extend_from_slice(&a.1[i..]);
        ids.extend_from_slice(&b.0[j..]);
        tags.extend_from_slice(&b.1[j..]);
        (ids, tags)
    }

    pub(super) fn add_wave(&mut self, rows: &[i64], k: usize, ids: Vec<u32>) {
        let t = self.fanout;
        while self.runs.len() >= t && self.runs[self.runs.len() - t].0.len() <= t * self.runs[self.runs.len() - 1].0.len() {
            // Merged two at a time, the newest first (fanout 2 is the measured one).
            let mut parts = self.runs.split_off(self.runs.len() - t);
            let mut acc = parts.pop().expect("a run");
            while let Some(p) = parts.pop() {
                self.merged += p.0.len() + acc.0.len();
                acc = self.merge2(rows, k, p, acc);
            }
            self.runs.push(acc);
        }
        if ids.is_empty() {
            return;
        }
        let lead = self.order[0];
        let mut tagged: Vec<(u16, u32)> = ids.iter().map(|&id| (tag(rows[id as usize * k + lead]), id)).collect();
        let order = &self.order;
        tagged.sort_unstable_by(|a, b| a.0.cmp(&b.0).then_with(|| Self::rows_cmp(order, rows, k, a.1, b.1)));
        self.runs.push((tagged.iter().map(|x| x.1).collect(), tagged.iter().map(|x| x.0).collect()));
    }

    /// Every run merged into one.
    pub(super) fn merged_all(&self, rows: &[i64], k: usize) -> TaggedRuns {
        let mut one = TaggedRuns { order: self.order.clone(), runs: Vec::new(), merged: 0, row_compares: 0, collisions: 0, fanout: self.fanout };
        let mut acc: Option<(Vec<u32>, Vec<u16>)> = None;
        for r in &self.runs {
            acc = Some(match acc.take() { None => r.clone(), Some(a) => one.merge2(rows, k, a, r.clone()) });
        }
        one.runs = acc.into_iter().collect();
        one
    }

    /// The number of ids holding `key` on the order's first `key.len()` columns, over every run.
    pub(super) fn count(&self, rows: &[i64], k: usize, key: &[i64]) -> usize {
        let t = tag(key[0]);
        let cmp = |id: u32| -> Ordering {
            let r = &rows[id as usize * k..][..k];
            self.order.iter().zip(key).map(|(&c, v)| r[c].cmp(v)).find(|o| o.is_ne()).unwrap_or(Ordering::Equal)
        };
        self.runs.iter().map(|(ids, tags)| {
            let lo = tags.partition_point(|&x| x < t);
            let hi = lo + tags[lo..].partition_point(|&x| x == t);
            let run = &ids[lo..hi];
            let s = run.partition_point(|&id| cmp(id) == Ordering::Less);
            run[s..].partition_point(|&id| cmp(id) == Ordering::Equal)
        }).sum()
    }

    /// `count`, each run's tag range found from where the tag would sit were the tags even (mldb's PathIndex: the
    /// position is the hash times the length), widened by doubling steps: a short scan, not a search of the run.
    pub(super) fn count_interp(&self, rows: &[i64], k: usize, key: &[i64]) -> usize {
        let t = tag(key[0]);
        let cmp = |id: u32| -> Ordering {
            let r = &rows[id as usize * k..][..k];
            self.order.iter().zip(key).map(|(&c, v)| r[c].cmp(v)).find(|o| o.is_ne()).unwrap_or(Ordering::Equal)
        };
        self.runs.iter().map(|(ids, tags)| {
            let (lo, hi) = interp_range(tags, t);
            let run = &ids[lo..hi];
            let s = run.partition_point(|&id| cmp(id) == Ordering::Less);
            run[s..].partition_point(|&id| cmp(id) == Ordering::Equal)
        }).sum()
    }

    pub(super) fn bytes(&self) -> usize {
        self.runs.iter().map(|r| r.0.capacity() * 4 + r.1.capacity() * 2).sum()
    }
}

#[cfg(test)]
pub mod grow_tests {
    use super::*;

    #[test]
    fn growing_indexes_dedup_and_find_what_a_whole_build_does() {
        let k = 3;
        // Rows 0..2000, then 500 that repeat earlier rows (the duplicates a wave offers).
        let mut rows: Vec<i64> = (0..2000i64).flat_map(|i| [i % 10, i % 7, i]).collect();
        rows.extend((0..500i64).flat_map(|i| [(i * 3) % 10, (i * 3) % 7, i * 3]));
        // 12 id bits leave 16 fingerprint bits; 26 leave 2, so the doublings read rows again.
        for id_bits in [12, 26] {
            let mut set = Growing::new(vec![0, 1, 2], id_bits);
            let mut by01 = Growing::new(vec![0, 1], id_bits);
            let mut runs = Runs::new(vec![0, 1, 2]);
            let mut wide = Runs::with_fanout(vec![0, 1, 2], 4);
            let mut tagged = TaggedRuns::with_fanout(vec![0, 1, 2], 2);
            let mut lazy = Runs::with_fanout(vec![0, 1, 2], usize::MAX);
            let mut held = 0;
            for wave in (0..2500u32).collect::<Vec<_>>().chunks(97) {
                set.reserve(&rows, k, wave.len());
                let mut fresh = Vec::new();
                for &id in wave {
                    let r = &rows[id as usize * k..][..k];
                    if set.insert(&rows, k, id, key_hash(r.iter().copied()), true) {
                        fresh.push(id);
                    }
                }
                by01.reserve(&rows, k, fresh.len());
                for &id in &fresh {
                    by01.insert(&rows, k, id, key_hash([rows[id as usize * k], rows[id as usize * k + 1]].into_iter()), false);
                }
                held += fresh.len();
                wide.add_wave(&rows, k, fresh.clone());
                tagged.add_wave(&rows, k, fresh.clone());
                if !fresh.is_empty() { lazy.add_wave(&rows, k, fresh.clone()); }
                runs.add_wave(&rows, k, fresh);
            }
            assert_eq!(held, 2000, "id bits {id_bits}");
            assert!(set.grew > 0 && runs.runs.len() < 12, "{} grew, {} runs", set.grew, runs.runs.len());
            // With 16 fingerprint bits only the rows whose distance saturated are read again; with 2, a doubling
            // reads them all every other time.
            assert!(if id_bits == 26 { set.rehashed > 1000 } else { set.rehashed < 100 }, "{} rehashed, id bits {id_bits}", set.rehashed);
            assert_eq!(runs.count(&rows, k, &[3]), 200);
            assert_eq!(runs.count(&rows, k, &[3, 4]), 28);
            assert_eq!(wide.count(&rows, k, &[3, 4]), 28);
            assert_eq!(wide.merged_all(&rows, k).count(&rows, k, &[3]), 200);
            assert!(wide.merged < runs.merged, "{} merged by 4s, {} by 2s", wide.merged, runs.merged);
            assert_eq!(tagged.count(&rows, k, &[3, 4]), 28);
            assert_eq!(tagged.count(&rows, k, &[3]), 200);
            assert_eq!(tagged.count_interp(&rows, k, &[3]), 200);
            assert_eq!(tagged.count_interp(&rows, k, &[3, 4]), 28);
            assert_eq!(tagged.count_interp(&rows, k, &[11]), 0);
            assert_eq!(tagged.merged_all(&rows, k).count(&rows, k, &[5, 5]), 29);
            assert_eq!(lazy.runs.len(), 21, "one run a wave of new rows, none merged");
            assert_eq!(lazy.count(&rows, k, &[3, 4]), 28);
            let mut got = 0;
            by01.each(&rows, k, &[3, 4], |id| { assert_eq!(&rows[id as usize * k..][..2], &[3, 4]); got += 1 });
            assert_eq!(got, 28, "id bits {id_bits}");
            for id in 0..2000u32 {
                assert!(set.insert_probe(&rows, k, id, key_hash(rows[id as usize * k..][..k].iter().copied())), "row {id} id bits {id_bits}");
            }
        }
    }
}
