//! A sorted order's search, written once (t56 step 3): the ids of a relation sorted by some of its columns, and beside
//! them, in the relation's block (`relation::Relation::try_add_order`, decision 0478 step 5b), the leading value of
//! every `FENCE`th id (`fences`), or each id's 16-bit tag of it (`tags`, an order sorted by its lead's hash), and each
//! id's 8-bit print of its whole key (`prints`, an order probed by whole keys). The set engine's runs
//! (`setfix::runs::Run`) and the batch's relations (`facts::vtab::Relation`) both search their orders here.
//!
//! A probe by a whole key reads the fences (or tags), then the span's prints, eight a word, and then only the rows
//! whose print is the key's: about one row read a probe, where a bisection read about eight, each a decode of the
//! row's sealed chunk.

use std::cmp::Ordering;

/// Ids between fences.
pub const FENCE: usize = 64;

/// 16 bits of a value's hash (Fibonacci hashing: the product's high bits).
#[inline]
pub fn tag(v: i64) -> u16 {
    ((v as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 48) as u16
}

/// A key's print: 8 bits of a hash of its values in order (another multiplier than `tag`'s, so a tagged order's span
/// of one tag still spreads over the prints).
#[inline]
pub fn print_vals(vals: impl Iterator<Item = i64>) -> u8 {
    let h = vals.fold(0x2545_F491_4F6C_DD1Du64, |h, v| (h ^ v as u64).wrapping_mul(0xD6E8_FEB8_6659_FD93));
    (h >> 56) as u8
}

/// A whole key's print (its values in the order's column order), as its rows' are.
#[inline]
pub fn print_of_key(key: &[i64]) -> u8 {
    print_vals(key.iter().copied())
}

/// Each byte of `w` equal to `b`, as its top bit (the lowest set bit marks the first exactly).
#[inline]
fn bytes_eq(w: u64, b: u8) -> u64 {
    let x = w ^ (b as u64 * 0x0101_0101_0101_0101);
    x.wrapping_sub(0x0101_0101_0101_0101) & !x & 0x8080_8080_8080_8080
}

/// The word of `prints` at `at` (bytes past `to` read as `pad`): eight prints a word.
#[inline]
fn word(prints: &[u8], at: usize, to: usize, pad: u8) -> u64 {
    if at + 8 <= to {
        return u64::from_le_bytes(prints[at..at + 8].try_into().expect("eight prints"));
    }
    let mut b = [pad; 8];
    b[..to - at].copy_from_slice(&prints[at..to]);
    u64::from_le_bytes(b)
}

/// The first position in `from..to` whose print is `p` (`eq`) or is not (`!eq`); `to` if none.
#[inline]
fn first_print(prints: &[u8], from: usize, to: usize, p: u8, eq: bool) -> usize {
    let mut at = from;
    while at < to {
        let w = word(prints, at, to, if eq { !p } else { p });
        let m = if eq { bytes_eq(w, p) } else { !bytes_eq(w, p) & 0x8080_8080_8080_8080 };
        if m != 0 {
            return (at + (m.trailing_zeros() / 8) as usize).min(to);
        }
        at += 8;
    }
    to
}

/// The span of positions `lo..hi` whose ids hold the whole key printed `p` (`is_key` reads a position's row): the
/// first print `p` that is the key starts it, and it ends at the last such before the prints stop being `p`. A
/// key's ids are next to each other in the order, so only prints equal by chance (one in 256) are read besides
/// the span's ends.
#[inline]
pub fn print_span(prints: &[u8], lo: usize, hi: usize, p: u8, is_key: impl Fn(usize) -> bool) -> (usize, usize) {
    let mut at = lo;
    let s = loop {
        let q = first_print(prints, at, hi, p, true);
        if q >= hi {
            return (hi, hi);
        }
        if is_key(q) {
            break q;
        }
        at = q + 1;
    };
    let mut e = first_print(prints, s + 1, hi, p, false);
    while e > s + 1 && !is_key(e - 1) {
        e -= 1;
    }
    (s, e)
}

/// The first position of `v` (partitioned: `less` holds before it, not from it) not `less`: a galloping search from
/// `start`, forward or back, so its cost grows with the distance from `start`, not with `v`'s length.
#[inline]
pub fn gallop<T>(v: &[T], start: usize, less: impl Fn(&T) -> bool) -> usize {
    let n = v.len();
    let start = start.min(n);
    if start < n && less(&v[start]) {
        // Forward: the first not `less` is past `start`.
        let mut step = 1;
        let mut lo = start + 1;
        while lo + step <= n && less(&v[lo + step - 1]) {
            lo += step;
            step *= 2;
        }
        let hi = (lo + step).min(n);
        lo + v[lo..hi].partition_point(|x| less(x))
    } else {
        // Back: the first not `less` is at or before `start`.
        let mut step = 1;
        let mut hi = start;
        while hi >= step && !less(&v[hi - step]) {
            hi -= step;
            step *= 2;
        }
        let lo = hi.saturating_sub(step);
        lo + v[lo..hi].partition_point(|x| less(x))
    }
}

/// The span of `tags` (sorted) equal to `t`, at or past `from`: its start by a galloping search from `from`, or (with
/// `place`) from where an even spread of tags would put it, back or forward; its end by a galloping search from its
/// start. A span costs reads logarithmic in its length: its end was walked, so a tag thousands of rows share (a leading
/// value most rows hold) was read to its end once per row a sorted batch searched. Generic so a test counts the reads.
#[inline]
pub fn tag_span<T: Copy + Ord + Into<u64>>(tags: &[T], t: T, from: usize, place: bool) -> (usize, usize) {
    let n = tags.len();
    if from >= n {
        return (n, n);
    }
    let at = if place { from.max(((t.into() as usize) * n) >> 16) } else { from };
    let s = from + gallop(&tags[from..], at - from, |&x| x < t);
    (s, s + gallop(&tags[s..], 0, |&x| x == t))
}

/// The first position in `lo..hi` where `less` stops holding (it holds before it, not from it).
#[inline]
pub fn split_at_pos(mut lo: usize, mut hi: usize, less: impl Fn(usize) -> bool) -> usize {
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if less(mid) { lo = mid + 1 } else { hi = mid }
    }
    lo
}

/// The positions of an order of `n` ids that hold leading value `lead`, by its fences: from the block before the
/// first fence at or past it, to the block of the first fence past it.
#[inline]
pub fn fence_span(fences: &[i64], n: usize, lead: i64) -> (usize, usize) {
    let a = fences.partition_point(|&f| f < lead);
    let b = a + fences[a..].partition_point(|&f| f == lead);
    (a.saturating_sub(1) * FENCE, (b * FENCE).min(n))
}

/// The positions of an order of `n` ids whose rows hold `key` on the order's first `key.len()` columns (`key` not
/// empty): `cmp(i)` compares position `i`'s row with the key. The span by the tags (a tagged order) or the fences;
/// within it, by the prints where they are kept and the key is whole (`whole`), else by bisection.
#[inline]
pub fn find(n: usize, fences: &[i64], tags: &[u16], prints: &[u8], key: &[i64], whole: bool, cmp: impl Fn(usize) -> Ordering) -> (usize, usize) {
    let lead = key[0];
    let (lo, hi) = if !tags.is_empty() { tag_span(tags, tag(lead), 0, true) } else { fence_span(fences, n, lead) };
    if !prints.is_empty() && whole {
        return print_span(prints, lo, hi, print_of_key(key), |i| cmp(i).is_eq());
    }
    let s = split_at_pos(lo, hi, |i| cmp(i) == Ordering::Less);
    let e = split_at_pos(s, hi, |i| cmp(i) == Ordering::Equal);
    (s, e)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An order of runs of equal keys, searched by fences alone and by fences and prints, finds each key's span as a
    /// scan does, for keys present and absent, and keys whose rows cross fences.
    #[test]
    fn an_orders_search_finds_a_scans_span() {
        // Values 0, 0, 0, 3, 3, 6, ...: each value 3k, k % 5 + 1 times; long runs cross fences.
        let mut vals: Vec<i64> = Vec::new();
        for k in 0..400i64 {
            for _ in 0..(k % 5 + 1 + if k % 37 == 0 { 150 } else { 0 }) {
                vals.push(3 * k);
            }
        }
        let n = vals.len();
        let fences: Vec<i64> = vals.iter().step_by(FENCE).copied().collect();
        let prints: Vec<u8> = vals.iter().map(|&v| print_vals(std::iter::once(v))).collect();
        for key in -2..1210i64 {
            let want = (vals.partition_point(|&v| v < key), vals.partition_point(|&v| v <= key));
            let cmp = |i: usize| vals[i].cmp(&key);
            let (s, e) = find(n, &fences, &[], &[], &[key], true, cmp);
            assert_eq!((s, e), want, "fences, key {key}");
            let (s, e) = find(n, &fences, &[], &prints, &[key], true, cmp);
            if want.0 == want.1 {
                assert_eq!(s, e, "prints, key {key} (absent)");
            } else {
                assert_eq!((s, e), want, "prints, key {key}");
            }
        }
    }
}
