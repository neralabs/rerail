//! Spike: the timed strata as sort, merge and adjacent dedup, with no hashing.
//!
//! The data is the device test's (`setfix::device::tests::joins_timed_on_the_cpu_and_the_device`): the same
//! generator, the same rows. A row is packed into one `u64` key at its columns' widths (a relation's column statistics
//! give them: its minimum and maximum), so a sort moves whole rows and a compare is one integer compare.
//!
//! - **Keyed join** `h0(x, z) :- a(x, y), b(y, z), z < x`: `a` and `b` sorted by `y` (radix), merged; each key's
//!   `a` range crossed with its `b` range, `z < x` tested, rows `(x, z)` out; those sorted by `(x, z)` and made
//!   distinct by comparing each with the one before.
//! - **Closure** `h0(x, z) :- h0(x, y), e(y, z)`: semi-naive rounds. A round's delta is sorted by `y` and merged with
//!   `e` sorted by its first column; the new rows are sorted, made distinct, and merged against `h0` (sorted by its
//!   row) to keep the rows it lacks, which are the next delta; `h0` takes them by a merge.
//!
//! Not the engine's: a timing of the shape on one thread, against the join program's (decision 0674 notes).

/// The device test's generator: `x = x * 6364136223846793005 + 1442695040888963407`, a value `(x >> 33) mod m`.
struct Gen(u64);

impl Gen {
    fn next(&mut self, m: i64) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 33) as i64).rem_euclid(m) as u64
    }
}

/// The test's relations, as `(c0, c1)` rows in insertion order: `a`, `b` and `e`.
struct Data {
    a: Vec<(u64, u64)>,
    b: Vec<(u64, u64)>,
    e: Vec<(u64, u64)>,
}

fn data() -> Data {
    let mut g = Gen(5);
    let (mut a, mut b, mut e) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..100_000 {
        let r = (g.next(1_000_000), g.next(20_000));
        a.push(r);
        let r = (g.next(20_000), g.next(1_000_000));
        b.push(r);
    }
    for v in 0..400u64 {
        e.push((v, v + 1));
        if g.next(4) == 0 {
            e.push((v, g.next(400)));
        }
    }
    Data { a, b, e }
}

/// Bits a column's values need (its maximum's).
fn bits(max: u64) -> u32 {
    64 - max.leading_zeros()
}

/// A stable LSD radix sort of `keys` by their bits `shift..shift + width`, digits of at most 11 bits (a histogram
/// of 2,048 counts, in L1), every pass's histogram counted in one read; a pass whose digit is the same for every key
/// is skipped. `tmp` is scratch of any length.
pub(crate) fn radix_sort(keys: &mut Vec<u64>, tmp: &mut Vec<u64>, shift: u32, width: u32) {
    let n = keys.len();
    if n < 2 || width == 0 {
        return;
    }
    let passes = width.div_ceil(11);
    let d = width.div_ceil(passes);
    let mask = (1u64 << d) - 1;
    let mut hist = vec![[0u32; 2048]; passes as usize];
    for &k in keys.iter() {
        for (p, h) in hist.iter_mut().enumerate() {
            h[((k >> (shift + p as u32 * d)) & mask) as usize] += 1;
        }
    }
    tmp.clear();
    tmp.resize(n, 0);
    for (p, h) in hist.iter_mut().enumerate() {
        if h.iter().any(|&c| c as usize == n) {
            continue;
        }
        let mut sum = 0u32;
        for c in h.iter_mut() {
            let v = *c;
            *c = sum;
            sum += v;
        }
        let sh = shift + p as u32 * d;
        for &k in keys.iter() {
            let b = ((k >> sh) & mask) as usize;
            // SAFETY: `h[b]` counts the keys before this one of its digit: below `n`.
            unsafe { *tmp.get_unchecked_mut(h[b] as usize) = k };
            h[b] += 1;
        }
        std::mem::swap(keys, tmp);
    }
}

/// `keys`, sorted, with each key equal to the one before it dropped, in place.
pub(crate) fn dedup_sorted(keys: &mut Vec<u64>) {
    if keys.is_empty() {
        return;
    }
    let mut w = 1usize;
    for r in 1..keys.len() {
        let k = keys[r];
        // Written always, kept where it differs from the last kept: no branch on the data.
        keys[w] = k;
        w += usize::from(k != keys[w - 1]);
    }
    keys.truncate(w);
}

/// The keyed join's rows, distinct, sorted by `(x, z)`: `a` (`y << xw | x`) and `b` (`y << zw | z`), each sorted by
/// `y`, merged.
fn merge_keyed(a: &[u64], xw: u32, b: &[u64], zw: u32, out: &mut Vec<u64>) {
    out.clear();
    let (xm, zm) = ((1u64 << xw) - 1, (1u64 << zw) - 1);
    let (mut i, mut j) = (0usize, 0usize);
    while i < a.len() && j < b.len() {
        let (ya, yb) = (a[i] >> xw, b[j] >> zw);
        if ya < yb {
            i += 1;
            continue;
        }
        if ya > yb {
            j += 1;
            continue;
        }
        let i1 = i + a[i..].iter().take_while(|&&k| k >> xw == ya).count();
        let j1 = j + b[j..].iter().take_while(|&&k| k >> zw == yb).count();
        let bz = &b[j..j1];
        out.reserve((i1 - i) * bz.len());
        for &ak in &a[i..i1] {
            let x = ak & xm;
            let at = out.len();
            // SAFETY: reserved for every pair of the key's ranges; the length counts those written.
            unsafe {
                let p = out.as_mut_ptr().add(at);
                let mut w = 0usize;
                for &bk in bz {
                    let z = bk & zm;
                    *p.add(w) = x << zw | z;
                    w += usize::from(z < x);
                }
                out.set_len(at + w);
            }
        }
        (i, j) = (i1, j1);
    }
}

/// `new` (sorted, distinct) less `old` (sorted, distinct): the rows of `new` that `old` lacks, by one merge.
pub(crate) fn sorted_minus(new: &[u64], old: &[u64], out: &mut Vec<u64>) {
    out.clear();
    out.reserve(new.len());
    let mut j = 0usize;
    for &k in new {
        while j < old.len() && old[j] < k {
            j += 1;
        }
        if j >= old.len() || old[j] != k {
            out.push(k);
        }
    }
}

/// `a` and `b` (sorted, disjoint) merged into `out`.
pub(crate) fn sorted_union(a: &[u64], b: &[u64], out: &mut Vec<u64>) {
    out.clear();
    out.reserve(a.len() + b.len());
    let (mut i, mut j) = (0usize, 0usize);
    while i < a.len() && j < b.len() {
        if a[i] < b[j] {
            out.push(a[i]);
            i += 1;
        } else {
            out.push(b[j]);
            j += 1;
        }
    }
    out.extend_from_slice(&a[i..]);
    out.extend_from_slice(&b[j..]);
}

/// The keyed join's times (ms) and rows: `b` sorted by `y` (once, as a relation sorted in place would be), then `a`
/// sorted, the merge, the output's sort and dedup.
pub(crate) struct Keyed {
    pub sort_b: f64,
    pub sort_a: f64,
    pub merge: f64,
    pub sort_out: f64,
    pub dedup: f64,
    pub rows: usize,
    pub pairs: usize,
}

fn keyed(d: &Data) -> Keyed {
    let t = std::time::Instant::now;
    let ms = |s: std::time::Instant| s.elapsed().as_secs_f64() * 1e3;
    // Column widths from the relations' maxima.
    let (xw, yw) = (bits(d.a.iter().map(|r| r.0).max().unwrap()), bits(d.a.iter().map(|r| r.1).max().unwrap()));
    let zw = bits(d.b.iter().map(|r| r.1).max().unwrap());
    let mut tmp = Vec::new();
    let s = t();
    let mut b: Vec<u64> = d.b.iter().map(|&(y, z)| y << zw | z).collect();
    radix_sort(&mut b, &mut tmp, zw, yw);
    let sort_b = ms(s);
    let s = t();
    let mut a: Vec<u64> = d.a.iter().map(|&(x, y)| y << xw | x).collect();
    radix_sort(&mut a, &mut tmp, xw, yw);
    let sort_a = ms(s);
    let s = t();
    let mut out = Vec::new();
    merge_keyed(&a, xw, &b, zw, &mut out);
    let merge = ms(s);
    let pairs = out.len();
    let s = t();
    radix_sort(&mut out, &mut tmp, 0, xw + zw);
    let sort_out = ms(s);
    let s = t();
    dedup_sorted(&mut out);
    let dedup = ms(s);
    Keyed { sort_b, sort_a, merge, sort_out, dedup, rows: out.len(), pairs }
}

/// The closure's times (ms) and rows: `e` sorted once, then the rounds; each round's parts summed over the rounds.
pub(crate) struct Closure {
    pub sort_e: f64,
    pub sort_delta: f64,
    pub merge: f64,
    pub sort_new: f64,
    pub minus: f64,
    pub union: f64,
    pub rounds: usize,
    pub rows: usize,
}

fn closure(d: &Data) -> Closure {
    let t = std::time::Instant::now;
    let ms = |s: std::time::Instant| s.elapsed().as_secs_f64() * 1e3;
    let w = bits(d.e.iter().map(|r| r.0.max(r.1)).max().unwrap());
    let m = (1u64 << w) - 1;
    let mut tmp = Vec::new();
    let mut c = Closure { sort_e: 0.0, sort_delta: 0.0, merge: 0.0, sort_new: 0.0, minus: 0.0, union: 0.0, rounds: 0, rows: 0 };
    // `e` by its row (`src << w | dst`): its first column leads, so it is sorted by the join key too.
    let s = t();
    let mut e: Vec<u64> = d.e.iter().map(|&(s, t)| s << w | t).collect();
    radix_sort(&mut e, &mut tmp, 0, 2 * w);
    dedup_sorted(&mut e);
    c.sort_e = ms(s);
    // The seed: h0 = e. Rows `x << w | y`, sorted.
    let mut h0 = e.clone();
    let mut delta = h0.clone();
    let (mut by_y, mut new, mut fresh, mut merged) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    while !delta.is_empty() {
        c.rounds += 1;
        // The delta by its join column: `y << w | x`.
        let s = t();
        by_y.clear();
        by_y.extend(delta.iter().map(|&k| (k & m) << w | k >> w));
        radix_sort(&mut by_y, &mut tmp, w, w);
        c.sort_delta += ms(s);
        // Merged with e: each y's delta rows crossed with its edges, rows `x << w | z`.
        let s = t();
        new.clear();
        let (mut i, mut j) = (0usize, 0usize);
        while i < by_y.len() && j < e.len() {
            let (ya, yb) = (by_y[i] >> w, e[j] >> w);
            if ya < yb {
                i += 1;
                continue;
            }
            if ya > yb {
                j += 1;
                continue;
            }
            let i1 = i + by_y[i..].iter().take_while(|&&k| k >> w == ya).count();
            let j1 = j + e[j..].iter().take_while(|&&k| k >> w == yb).count();
            for &dk in &by_y[i..i1] {
                let x = dk & m;
                new.extend(e[j..j1].iter().map(|&ek| x << w | (ek & m)));
            }
            (i, j) = (i1, j1);
        }
        c.merge += ms(s);
        let s = t();
        radix_sort(&mut new, &mut tmp, 0, 2 * w);
        dedup_sorted(&mut new);
        c.sort_new += ms(s);
        let s = t();
        sorted_minus(&new, &h0, &mut fresh);
        c.minus += ms(s);
        let s = t();
        sorted_union(&h0, &fresh, &mut merged);
        std::mem::swap(&mut h0, &mut merged);
        c.union += ms(s);
        std::mem::swap(&mut delta, &mut fresh);
    }
    c.rows = h0.len();
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    /// The keyed join and the closure come out as a plain nested-loop and semi-naive evaluation of the same data has
    /// them, and as the engine's runs counted them (249,484 and 157,228 rows).
    #[test]
    fn the_spike_makes_the_engines_rows() {
        let d = data();
        let mut by_y: HashMap<u64, Vec<u64>> = HashMap::new();
        for &(y, z) in &d.b {
            by_y.entry(y).or_default().push(z);
        }
        let mut want = HashSet::new();
        for &(x, y) in &d.a {
            for &z in by_y.get(&y).map_or(&[][..], |v| v) {
                if z < x {
                    want.insert((x, z));
                }
            }
        }
        assert_eq!(want.len(), 249_484);
        assert_eq!(keyed(&d).rows, want.len());
        let mut h: HashSet<(u64, u64)> = d.e.iter().copied().collect();
        let mut delta: Vec<(u64, u64)> = h.iter().copied().collect();
        while !delta.is_empty() {
            let mut next = Vec::new();
            for &(x, y) in &delta {
                for &(s, t) in &d.e {
                    if s == y && h.insert((x, t)) {
                        next.push((x, t));
                    }
                }
            }
            delta = next;
        }
        assert_eq!(h.len(), 157_228);
        assert_eq!(closure(&d).rows, h.len());
    }

    /// The spike's times, the best of 20 runs each (a measure: `--ignored`, at opt-level 3, pinned to one core).
    #[test]
    #[ignore]
    fn the_spike_timed() {
        let d = data();
        let mut best_k: Option<Keyed> = None;
        let total_k = |k: &Keyed| k.sort_a + k.merge + k.sort_out + k.dedup;
        for _ in 0..20 {
            let k = keyed(&d);
            if best_k.as_ref().is_none_or(|b| total_k(&k) < total_k(b)) {
                best_k = Some(k);
            }
        }
        let k = best_k.unwrap();
        eprintln!(
            "keyed: {} pairs, {} rows; sort b {:.2} ms (once), sort a {:.2}, merge {:.2}, sort out {:.2}, dedup {:.2}: {:.2} ms",
            k.pairs, k.rows, k.sort_b, k.sort_a, k.merge, k.sort_out, k.dedup, total_k(&k)
        );
        let mut best_c: Option<Closure> = None;
        let total_c = |c: &Closure| c.sort_delta + c.merge + c.sort_new + c.minus + c.union;
        for _ in 0..20 {
            let c = closure(&d);
            if best_c.as_ref().is_none_or(|b| total_c(&c) < total_c(b)) {
                best_c = Some(c);
            }
        }
        let c = best_c.unwrap();
        eprintln!(
            "closure: {} rounds, {} rows; sort e {:.3} ms (once), sort delta {:.2}, merge {:.2}, sort new {:.2}, minus {:.2}, union {:.2}: {:.2} ms",
            c.rounds, c.rows, c.sort_e, c.sort_delta, c.merge, c.sort_new, c.minus, c.union, total_c(&c)
        );
    }
}
