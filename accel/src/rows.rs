//! Rows ordered by a key column on the host (decision 0095b §4): the CPU lane's joins, and the narrow form a column
//! takes on the device. Moved into the executor with the device's joins (decision 0100 §1): the CPU reference of
//! the `join` kernel is `probe` here.
//!
//! - **The span.** A column's least and greatest value, in one pass of eight lanes at a time (it vectorizes). A key
//!   is sorted as its offset from the least, `v - min`: only `bits(max - min)` bits of it are sorted, and a
//!   column is held on the device in 16, 32 or 64 bits of offset, exactly.
//! - **The sort.** An LSD radix sort, stable, of the offsets and the rows' numbers packed in one word: `offset << 32
//!   | row` in a `u64` when the offsets fit 32 bits, else a `u128`. Eight-bit digits; every digit's histogram is
//!   taken in one pass over the rows, and a digit every row shares is not a pass at all. Fewer than `SMALL` rows
//!   are sorted by comparison of the same packed words.
//! - **The runs.** The sorted keys' runs of one value: each value, its first place and its count.
//! - **The join.** The equi-join of two relations on their key columns: both sorted by key 0, the runs merged, each
//!   pair of a matched run's rows tested on the other keys. Its pairs, (left row, right row), are in key order.
//! - **The probe.** The same pairs with only the right sorted: each left row, in order, finds its key's run by a
//!   binary search of the runs and tests the run's rows on the other keys. Its pairs are in left row order, and
//!   within a left row in right row order (the sort is stable): the device's probe's order, exactly.

/// Below this many rows a comparison sort of the packed words is the faster.
const SMALL: usize = 2048;

/// A column's least and greatest value (`None` when it has none).
pub fn span(values: &[i64]) -> Option<(i64, i64)> {
    let first = *values.first()?;
    let (mut lo, mut hi) = ([first; 8], [first; 8]);
    let mut chunks = values.chunks_exact(8);
    for c in &mut chunks {
        for k in 0..8 {
            lo[k] = lo[k].min(c[k]);
            hi[k] = hi[k].max(c[k]);
        }
    }
    let rest = chunks.remainder();
    let lo = lo.into_iter().chain(rest.iter().copied()).min().unwrap_or(first);
    let hi = hi.into_iter().chain(rest.iter().copied()).max().unwrap_or(first);
    Some((lo, hi))
}

/// The bits of `max - min`, as an unsigned offset: 0 when every value is one.
pub fn bits(min: i64, max: i64) -> u32 {
    64 - (max.wrapping_sub(min) as u64).leading_zeros()
}

/// A word an LSD pass sorts: its 8-bit digit at `shift`.
trait Packed: Copy + Ord + Default {
    fn digit(self, shift: u32) -> usize;
    fn row(self) -> u32;
}

impl Packed for u64 {
    fn digit(self, shift: u32) -> usize {
        (self >> shift) as usize & 0xff
    }
    fn row(self) -> u32 {
        self as u32
    }
}

impl Packed for u128 {
    fn digit(self, shift: u32) -> usize {
        (self >> shift) as usize & 0xff
    }
    fn row(self) -> u32 {
        self as u32
    }
}

/// Sorts `words` by their bits `32..32 + bits` (the offsets; the row below them keeps it stable).
fn radix<T: Packed>(words: &mut Vec<T>, bits: u32) {
    if words.len() < SMALL {
        words.sort_unstable();
        return;
    }
    let passes = bits.div_ceil(8) as usize;
    // Every digit's histogram, in one pass over the rows.
    let mut hist = vec![[0u32; 256]; passes];
    for &w in words.iter() {
        for (p, h) in hist.iter_mut().enumerate() {
            h[w.digit(32 + 8 * p as u32)] += 1;
        }
    }
    let n = words.len() as u32;
    let mut other = vec![T::default(); words.len()];
    for (p, h) in hist.iter().enumerate() {
        // A digit every row shares orders nothing.
        if h.contains(&n) {
            continue;
        }
        let mut at = [0usize; 256];
        let mut sum = 0usize;
        for (d, &c) in h.iter().enumerate() {
            at[d] = sum;
            sum += c as usize;
        }
        let shift = 32 + 8 * p as u32;
        for &w in words.iter() {
            let d = w.digit(shift);
            other[at[d]] = w;
            at[d] += 1;
        }
        std::mem::swap(words, &mut other);
    }
}

/// Rows in the order of their keys, stable: each place's row, and each place's key.
pub struct Sorted {
    pub perm: Vec<u32>,
    pub keys: Vec<i64>,
}

/// The rows of `keys` in the keys' order, stable.
pub fn sort(keys: &[i64]) -> Sorted {
    let Some((min, max)) = span(keys) else { return Sorted { perm: Vec::new(), keys: Vec::new() } };
    let b = bits(min, max);
    let (perm, keys): (Vec<u32>, Vec<i64>) = if b <= 32 {
        let mut w: Vec<u64> = keys.iter().enumerate().map(|(r, &v)| (v.wrapping_sub(min) as u64) << 32 | r as u64).collect();
        radix(&mut w, b);
        w.iter().map(|&w| (w.row(), ((w >> 32) as i64).wrapping_add(min))).unzip()
    } else {
        let mut w: Vec<u128> = keys.iter().enumerate().map(|(r, &v)| u128::from(v.wrapping_sub(min) as u64) << 32 | r as u128).collect();
        radix(&mut w, b);
        w.iter().map(|&w| (w.row(), ((w >> 32) as u64 as i64).wrapping_add(min))).unzip()
    };
    Sorted { perm, keys }
}

/// The runs of sorted keys: each value, its first place and its count.
pub fn runs(sorted: &[i64]) -> Vec<(i64, u32, u32)> {
    let mut out: Vec<(i64, u32, u32)> = Vec::new();
    for (i, &k) in sorted.iter().enumerate() {
        match out.last_mut() {
            Some(last) if last.0 == k => last.2 += 1,
            _ => out.push((k, i as u32, 1)),
        }
    }
    out
}

/// A relation's rows sorted by its key 0, and the key's runs: what a join probes, kept while the relation holds.
pub struct Keyed {
    pub sorted: Sorted,
    pub runs: Vec<(i64, u32, u32)>,
}

impl Keyed {
    pub fn new(key: &[i64]) -> Keyed {
        let sorted = sort(key);
        let runs = runs(&sorted.keys);
        Keyed { sorted, runs }
    }
}

/// The equi-join of `left` and `right` on their columns (key `k` is each one's column `k`): the pairs of rows,
/// (left, right), in key order.
pub fn join(left: &[&[i64]], right: &[&[i64]]) -> Vec<(u32, u32)> {
    let Some(r0) = right.first() else { return Vec::new() };
    join_keyed(left, &Keyed::new(r0), right)
}

/// `join` with the right side sorted already (`right` its columns, `keyed` its key 0's order and runs): the left
/// sorted, the two run lists merged.
pub fn join_keyed(left: &[&[i64]], keyed: &Keyed, right: &[&[i64]]) -> Vec<(u32, u32)> {
    let Some(l0) = left.first() else { return Vec::new() };
    let ls = sort(l0);
    let lr = runs(&ls.keys);
    let (rr, rperm) = (&keyed.runs, &keyed.sorted.perm);
    let mut pairs = Vec::new();
    let mut r = 0;
    for &(key, start, count) in &lr {
        while r < rr.len() && rr[r].0 < key {
            r += 1;
        }
        let Some(&(rk, rstart, rcount)) = rr.get(r) else { break };
        if rk != key {
            continue;
        }
        let rows = &rperm[rstart as usize..(rstart + rcount) as usize];
        for &a in &ls.perm[start as usize..(start + count) as usize] {
            if left.len() == 1 {
                pairs.extend(rows.iter().map(|&b| (a, b)));
                continue;
            }
            for &b in rows {
                if (1..left.len()).all(|k| left[k][a as usize] == right[k][b as usize]) {
                    pairs.push((a, b));
                }
            }
        }
    }
    pairs
}

/// The equi-join of `left` and `right` (sorted by its key 0 already: `keyed`), in left row order and then right row
/// order: each left row's key found among the runs by binary search.
pub fn probe(left: &[&[i64]], keyed: &Keyed, right: &[&[i64]]) -> Vec<(u32, u32)> {
    let Some(l0) = left.first() else { return Vec::new() };
    let (runs, perm) = (&keyed.runs, &keyed.sorted.perm);
    let mut pairs = Vec::new();
    for (a, &key) in l0.iter().enumerate() {
        let Ok(r) = runs.binary_search_by(|run| run.0.cmp(&key)) else { continue };
        let (_, start, count) = runs[r];
        for &b in &perm[start as usize..(start + count) as usize] {
            if (1..left.len()).all(|k| left[k][a] == right[k][b as usize]) {
                pairs.push((a as u32, b));
            }
        }
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small deterministic generator (no dependency for a test's rows).
    fn rows(n: usize, seed: u64, modulus: i64, high: i64) -> Vec<i64> {
        let mut x = seed;
        (0..n)
            .map(|_| {
                x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                high + ((x >> 33) as i64).rem_euclid(modulus)
            })
            .collect()
    }

    #[test]
    fn the_radix_sort_is_a_stable_sort_and_the_join_a_hash_joins_pairs() {
        // Narrow offsets (one word), wide ones (two), negative values, one value, and the comparison sort's sizes.
        for (n, modulus, high) in [(10_000, 300, 0), (10_000, 1 << 40, -(1 << 50)), (5000, 1, 7), (100, 50, 1 << 33), (0, 1, 0)] {
            let keys = rows(n, 7, modulus, high);
            let got = sort(&keys);
            let mut want: Vec<u32> = (0..n as u32).collect();
            want.sort_by_key(|&r| keys[r as usize]);
            assert_eq!(got.perm, want, "n {n} modulus {modulus}");
            assert!(got.keys.iter().zip(&got.perm).all(|(&k, &r)| k == keys[r as usize]));
            assert_eq!(runs(&got.keys).iter().map(|r| r.2 as usize).sum::<usize>(), n);
        }
        let (a0, a1) = (rows(6000, 1, 900, 1 << 32), rows(6000, 2, 3, 0));
        let (b0, b1) = (rows(4000, 3, 900, 1 << 32), rows(4000, 4, 3, 0));
        let mut got = join(&[&a0, &a1], &[&b0, &b1]);
        got.sort_unstable();
        let mut want = Vec::new();
        for a in 0..a0.len() {
            for b in 0..b0.len() {
                if a0[a] == b0[b] && a1[a] == b1[b] {
                    want.push((a as u32, b as u32));
                }
            }
        }
        assert_eq!(got, want);
        // The probe: the same pairs, already in (left, right) order.
        assert_eq!(probe(&[&a0, &a1], &Keyed::new(&b0), &[&b0, &b1]), want);
        assert_eq!(span(&[3, -2, 9, 4, 4, 4, 4, 4, 4, 0]), Some((-2, 9)));
        assert_eq!(bits(5, 5), 0);
        assert_eq!(bits(i64::MIN, i64::MAX), 64);
    }
}
