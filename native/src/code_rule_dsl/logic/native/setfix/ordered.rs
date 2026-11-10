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
mod tests {
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
}
