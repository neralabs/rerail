//! Sorting for freeze and pack: a least-significant-digit radix sort of 64-bit keys, with 32-bit ids alongside.
//!
//! A canonical order compares rows column by column. Freeze packs a row's order key into one `u64` instead: each
//! column's value less the column's minimum, in the bits its range needs, the first column highest. The packed
//! keys sort as the rows do, and equal keys are equal rows. A radix sort of them reads and writes each key a few
//! times (one pass per 11 bits the keys use) and compares nothing: no branch on the data and no comparator call.

/// Bits a digit takes: 2,048 buckets a pass, so a pass's counts stay in L1.
const DIGIT: u32 = 11;
const BUCKETS: usize = 1 << DIGIT;

/// Sorts `keys`, with `ids` moved alongside, by their low `bits` bits, stably. `tk` and `ti` are room for as many
/// keys and ids.
pub fn radix_pairs(keys: &mut [u64], ids: &mut [u32], bits: u32, tk: &mut [u64], ti: &mut [u32]) {
    let n = keys.len();
    assert!(ids.len() == n && tk.len() >= n && ti.len() >= n);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radix_sorts_as_a_stable_sort_does() {
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        for bits in [1u32, 7, 11, 12, 33, 49, 64] {
            let mask = if bits == 64 { u64::MAX } else { (1u64 << bits) - 1 };
            let mut keys: Vec<u64> = (0..5_000)
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
            assert_eq!(got, want, "{bits} bits");
        }
    }
}
