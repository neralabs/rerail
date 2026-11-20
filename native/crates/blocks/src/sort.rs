//! Sorting for freeze and pack: a least-significant-digit radix sort of 64-bit keys, with 32-bit ids alongside.
//!
//! A canonical order compares rows column by column. Freeze packs a row's order key into one `u64` instead: each
//! column's value less the column's minimum, in the bits its range needs, the first column highest. The packed
//! keys sort as the rows do, and equal keys are equal rows. A radix sort of them reads and writes each key a few
//! times (one pass per 11 bits the keys use) and compares nothing: no branch on the data and no comparator call.
//!
//! A large sort runs on every thread: one pass by its top bits splits the keys into buckets of about `BUCKET`
//! keys (in parallel, by pieces), and each bucket is then sorted by the rest of its bits on its own, in cache.

/// Bits a digit takes: 2,048 buckets a pass, so a pass's counts stay in L1.
const DIGIT: u32 = 11;
const BUCKETS: usize = 1 << DIGIT;
/// Keys a sort takes before it runs in parallel.
const PAR_MIN: usize = 1 << 18;
/// Keys a piece of the parallel split reads.
const PIECE: usize = 1 << 16;
/// Keys a bucket of the parallel split holds, about: a bucket's sort stays in a core's cache.
const BUCKET: usize = 1 << 15;

/// Sorts `keys`, with `ids` moved alongside, by their low `bits` bits, stably. `tk` and `ti` are room for as many
/// keys and ids.
pub fn radix_pairs(keys: &mut [u64], ids: &mut [u32], bits: u32, tk: &mut [u64], ti: &mut [u32]) {
    let n = keys.len();
    assert!(ids.len() == n && tk.len() >= n && ti.len() >= n);
    if n >= PAR_MIN && bits > DIGIT {
        return split_par(keys, ids, bits, &mut tk[..n], &mut ti[..n]);
    }
    let passes = bits.div_ceil(DIGIT) as usize;
    if n < 2 || passes == 0 {
        return;
    }
    // Every pass's counts in one read of the keys (on the stack: 48 KB at most).
    let mut all = [[0u32; BUCKETS]; 64usize.div_ceil(DIGIT as usize)];
    let counts = &mut all[..passes];
    for &k in keys.iter() {
        for (p, c) in counts.iter_mut().enumerate() {
            c[((k >> (p as u32 * DIGIT)) as usize) & (BUCKETS - 1)] += 1;
        }
    }
    let (mut sk, mut si, mut dk, mut di) = (&mut keys[..], &mut ids[..], &mut tk[..n], &mut ti[..n]);
    let mut swapped = false;
    for (p, c) in counts.iter_mut().enumerate() {
        // A pass whose digit is the same for every key moves nothing.
        if c.iter().any(|&x| x as usize == n) {
            continue;
        }
        let mut at = 0u32;
        for x in c.iter_mut() {
            let here = *x;
            *x = at;
            at += here;
        }
        let shift = p as u32 * DIGIT;
        for (&k, &id) in sk.iter().zip(si.iter()) {
            let b = ((k >> shift) as usize) & (BUCKETS - 1);
            let to = c[b] as usize;
            c[b] += 1;
            dk[to] = k;
            di[to] = id;
        }
        std::mem::swap(&mut sk, &mut dk);
        std::mem::swap(&mut si, &mut di);
        swapped = !swapped;
    }
    if swapped {
        // The sorted run is in the room: copy it home.
        dk.copy_from_slice(sk);
        di.copy_from_slice(si);
    }
}

/// A pointer written through by several threads, at places no two of them share.
#[derive(Clone, Copy)]
struct Shared<T>(*mut T);
// SAFETY: every write through it is to a place one thread owns (the split's offsets are disjoint).
unsafe impl<T> Send for Shared<T> {}
unsafe impl<T> Sync for Shared<T> {}

/// A large sort on every thread. The top `tb` bits (enough for buckets of about `BUCKET` keys, at most a digit)
/// split the keys into `tk`, stably: each piece counts its top digits, the offsets run bucket by bucket and within
/// a bucket piece by piece, then each piece moves its keys to its offsets. Each bucket, its keys equal in the top
/// bits, is then sorted by the rest on its own (its place in `keys` the room) and copied home. When the low bits
/// are a whole number of digits, this is as many passes as one thread's sort; else one more.
fn split_par(keys: &mut [u64], ids: &mut [u32], bits: u32, tk: &mut [u64], ti: &mut [u32]) {
    use rayon::prelude::*;
    let n = keys.len();
    let want = (n / BUCKET).max(2).ilog2().min(DIGIT);
    // The low bits a whole number of digits, when that leaves the top between `want` bits and a digit.
    let low = ((bits - want) / DIGIT) * DIGIT;
    let low = if bits - low > DIGIT { bits - want } else { low };
    let nb = 1usize << (bits - low);
    let top = |k: u64| ((k >> low) as usize) & (nb - 1);
    let pieces = n.div_ceil(PIECE);
    let mut hist = vec![0u32; pieces * nb];
    rerail_context::on_cores(|| {
        hist.par_chunks_mut(nb).zip(keys.par_chunks(PIECE)).for_each(|(h, ks)| {
            for &k in ks {
                h[top(k)] += 1;
            }
        })
    });
    let mut starts = Vec::with_capacity(nb + 1);
    let mut at = 0u32;
    for b in 0..nb {
        starts.push(at as usize);
        for q in 0..pieces {
            let x = hist[q * nb + b];
            hist[q * nb + b] = at;
            at += x;
        }
    }
    starts.push(n);
    let (kp, ip) = (Shared(tk.as_mut_ptr()), Shared(ti.as_mut_ptr()));
    rerail_context::on_cores(|| {
        hist.par_chunks_mut(nb).zip(keys.par_chunks(PIECE).zip(ids.par_chunks(PIECE))).for_each(|(h, (ks, is))| {
            for (&k, &id) in ks.iter().zip(is) {
                let b = top(k);
                let to = h[b] as usize;
                h[b] += 1;
                // SAFETY: `to` is below `n` and this piece's alone (the offsets above).
                unsafe {
                    *{ kp }.0.add(to) = k;
                    *{ ip }.0.add(to) = id;
                }
            }
        })
    });
    // Each bucket by its low bits, with its place in `keys` as room, then home.
    let mut parts: Vec<(&mut [u64], &mut [u32], &mut [u64], &mut [u32])> = Vec::with_capacity(nb);
    let (mut k, mut i, mut rk, mut ri) = (&mut keys[..], &mut ids[..], &mut tk[..], &mut ti[..]);
    for w in starts.windows(2) {
        let len = w[1] - w[0];
        let (a, b) = std::mem::take(&mut k).split_at_mut(len);
        let (c, d) = std::mem::take(&mut i).split_at_mut(len);
        let (e, f) = std::mem::take(&mut rk).split_at_mut(len);
        let (g, h) = std::mem::take(&mut ri).split_at_mut(len);
        parts.push((a, c, e, g));
        (k, i, rk, ri) = (b, d, f, h);
    }
    rerail_context::on_cores(|| {
        parts.into_par_iter().for_each(|(home_k, home_i, sk, si)| {
            radix_pairs(sk, si, low, home_k, home_i);
            home_k.copy_from_slice(sk);
            home_i.copy_from_slice(si);
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radix_sorts_as_a_stable_sort_does() {
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        let cases = [(1u32, 5_000), (7, 5_000), (11, 5_000), (12, 5_000), (33, 5_000), (49, 5_000), (64, 5_000), (40, 300_000), (13, 300_000), (64, 300_000)];
        for (bits, n) in cases {
            let mask = if bits == 64 { u64::MAX } else { (1u64 << bits) - 1 };
            let mut keys: Vec<u64> = (0..n)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    x & mask & !0xF | (x & 3)
                })
                .collect();
            let mut ids: Vec<u32> = (0..keys.len() as u32).collect();
            let mut want: Vec<(u64, u32)> = keys.iter().copied().zip(ids.iter().copied()).collect();
            want.sort_by_key(|p| p.0);
            let (mut tk, mut ti) = (vec![0; keys.len()], vec![0; keys.len()]);
            radix_pairs(&mut keys, &mut ids, bits, &mut tk, &mut ti);
            let got: Vec<(u64, u32)> = keys.into_iter().zip(ids).collect();
            assert_eq!(got, want, "{bits} bits, {n} keys");
        }
    }

    #[test]
    fn a_skewed_parallel_sort_is_stable() {
        // Nine keys in ten in one top bucket: its sort is large enough to split again.
        let n = 600_000;
        let mut x: u64 = 0x2545_F491_4F6C_DD1D;
        let mut keys: Vec<u64> = (0..n)
            .map(|i| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                if i % 10 == 0 { x & ((1 << 40) - 1) } else { x & 0x0FFF_FFFF }
            })
            .collect();
        let mut ids: Vec<u32> = (0..n as u32).collect();
        let mut want: Vec<(u64, u32)> = keys.iter().copied().zip(ids.iter().copied()).collect();
        want.sort_by_key(|p| p.0);
        let (mut tk, mut ti) = (vec![0; n], vec![0; n]);
        radix_pairs(&mut keys, &mut ids, 40, &mut tk, &mut ti);
        assert_eq!(keys.into_iter().zip(ids).collect::<Vec<_>>(), want);
    }
}
