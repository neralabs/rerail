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

/// A key a sort moves: a row packed at its columns' widths, stored in the narrowest integer that holds it (`u32` where
/// it fits, else `u64`).
pub(crate) trait Key: Copy + Ord + Default + core::fmt::Debug {
    fn u(self) -> u64;
    fn of(v: u64) -> Self;
    /// The first place from `i` of `s` (sorted by `k >> sh`) whose `k >> sh` is past `t`, a vector of compares a step.
    fn past(s: &[Self], i: usize, t: u64, sh: u32) -> usize;
    /// `keys` (sorted) without repeats, into `out`, a vector a step.
    fn dedup_simd(keys: &[Self], out: &mut Vec<Self>);
}

impl Key for u64 {
    #[inline(always)]
    fn u(self) -> u64 {
        self
    }
    #[inline(always)]
    fn of(v: u64) -> u64 {
        v
    }
    fn past(s: &[u64], i: usize, t: u64, sh: u32) -> usize {
        vk::past(s, i, t, sh)
    }
    fn dedup_simd(keys: &[u64], out: &mut Vec<u64>) {
        dedup_simd(keys, out)
    }
}

impl Key for u32 {
    #[inline(always)]
    fn u(self) -> u64 {
        u64::from(self)
    }
    #[inline(always)]
    fn of(v: u64) -> u32 {
        v as u32
    }
    fn past(s: &[u32], i: usize, t: u64, sh: u32) -> usize {
        vk32::past(s, i, t as u32, sh)
    }
    fn dedup_simd(keys: &[u32], out: &mut Vec<u32>) {
        vk32::dedup(keys, out)
    }
}

/// A stable LSD radix sort of `keys` by their bits `shift..shift + width`, digits of at most 11 bits (a histogram
/// of 2,048 counts, in L1), every pass's histogram counted in one read; a pass whose digit is the same for every key
/// is skipped. `tmp` is scratch of any length.
pub(crate) fn radix_sort<K: Key>(keys: &mut Vec<K>, tmp: &mut Vec<K>, shift: u32, width: u32) {
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
            h[((k.u() >> (shift + p as u32 * d)) & mask) as usize] += 1;
        }
    }
    tmp.clear();
    tmp.resize(n, K::default());
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
            let b = ((k.u() >> sh) & mask) as usize;
            // SAFETY: `h[b]` counts the keys before this one of its digit: below `n`.
            unsafe { *tmp.get_unchecked_mut(h[b] as usize) = k };
            h[b] += 1;
        }
        std::mem::swap(keys, tmp);
    }
}

/// `keys`, sorted, with each key equal to the one before it dropped, in place.
pub(crate) fn dedup_sorted<K: Key>(keys: &mut Vec<K>) {
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
pub(crate) fn sorted_minus<K: Key>(new: &[K], old: &[K], out: &mut Vec<K>) {
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
pub(crate) fn sorted_union<K: Key>(a: &[K], b: &[K], out: &mut Vec<K>) {
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

/// `new` (sorted, distinct) less the rows of `run` (sorted): each searched for by galloping from where the last was
/// found, so a run many times longer than `new` costs about `new`'s length times the log of their ratio, not the
/// run's length. In place.
pub(crate) fn minus_gallop<K: Key>(new: &mut Vec<K>, run: &[K]) {
    let (mut j, mut w) = (0usize, 0usize);
    for r in 0..new.len() {
        let k = new[r];
        // The first place of `run` from `j` holding `k` or more.
        if j < run.len() && run[j] < k {
            let mut step = 1usize;
            let mut lo = j;
            while lo + step < run.len() && run[lo + step] < k {
                lo += step;
                step *= 2;
            }
            let hi = (lo + step).min(run.len());
            j = lo + 1 + run[lo + 1..hi].partition_point(|&v| v < k);
        }
        new[w] = k;
        w += usize::from(j >= run.len() || run[j] != k);
    }
    new.truncate(w);
}

/// `new` (sorted, distinct) less the rows of `run` (sorted), by one walk of both, with no branch on whether a row is
/// kept. In place.
pub(crate) fn minus_linear<K: Key>(new: &mut Vec<K>, run: &[K]) {
    let (n, m) = (new.len(), run.len());
    let (mut r, mut j, mut w) = (0usize, 0usize, 0usize);
    let p = new.as_mut_ptr();
    // SAFETY: `w <= r < n`, `j < m` in the loop. Each step: the lesser side steps, both where equal; `new`'s row is
    // written always and kept where it is below the run's.
    unsafe {
        while r < n && j < m {
            let (a, b) = (*p.add(r), *run.get_unchecked(j));
            *p.add(w) = a;
            w += usize::from(a < b);
            r += usize::from(a <= b);
            j += usize::from(a >= b);
        }
        while r < n {
            *p.add(w) = *p.add(r);
            w += 1;
            r += 1;
        }
    }
    new.truncate(w);
}

/// A relation's rows as sorted runs (decision 0112's orders, here of the rows themselves): a round's new rows are one
/// run; two runs merge where the older is at most twice the newer, so there are about log2 of the rows of them and
/// each row is merged about that many times in all.
pub(crate) struct Runs<K> {
    pub runs: Vec<Vec<K>>,
}

impl<K: Key> Runs<K> {
    /// `new` (sorted, distinct) less every run's rows: the rows the relation lacks.
    pub(crate) fn minus(&self, new: &mut Vec<K>) {
        for run in self.runs.iter().rev() {
            if new.is_empty() {
                return;
            }
            // A run at most a few times `new`'s length: one linear merge walks it for less than a search a row.
            if run.len() <= 4 * new.len() {
                minus_linear(new, run);
            } else {
                minus_gallop(new, run);
            }
        }
    }

    /// `fresh` (sorted, distinct, none held) as the newest run, the runs merged as their sizes say.
    pub(crate) fn add(&mut self, fresh: Vec<K>, simd: bool) {
        if fresh.is_empty() {
            return;
        }
        self.runs.push(fresh);
        while self.runs.len() >= 2 && self.runs[self.runs.len() - 2].len() <= 2 * self.runs[self.runs.len() - 1].len() {
            let b = self.runs.pop().unwrap();
            let a = self.runs.pop().unwrap();
            let mut out = Vec::new();
            if simd { union_branchless(&a, &b, &mut out) } else { sorted_union(&a, &b, &mut out) }
            self.runs.push(out);
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.runs.iter().map(Vec::len).sum()
    }
}

/// `a` and `b` (sorted, disjoint) merged, with no branch on the data: each step writes the lesser and steps that side.
pub(crate) fn union_branchless<K: Key>(a: &[K], b: &[K], out: &mut Vec<K>) {
    out.clear();
    out.reserve(a.len() + b.len());
    let (mut i, mut j, mut w) = (0usize, 0usize, 0usize);
    let p = out.as_mut_ptr();
    // SAFETY: `w` is `i + j`, below `a.len() + b.len()`, reserved; `i`, `j` within their slices in the loop.
    unsafe {
        while i < a.len() && j < b.len() {
            let (x, y) = (*a.get_unchecked(i), *b.get_unchecked(j));
            let t = y < x;
            *p.add(w) = if t { y } else { x };
            w += 1;
            i += usize::from(!t);
            j += usize::from(t);
        }
        out.set_len(w);
    }
    out.extend_from_slice(&a[i..]);
    out.extend_from_slice(&b[j..]);
}

/// The vector kit: 8 lanes of `u64` (four NEON registers).
pub(crate) mod vk {
    use rerail_simd::prelude::*;

    pub type V = Simd<u64, 8>;

    /// For each 8 lanes' bits, the bytes of 8 64-bit lanes that move the lanes of the bits to the front.
    pub const PACK64: [[u8; 64]; 256] = {
        let mut t = [[0u8; 64]; 256];
        let mut bits = 0;
        while bits < 256 {
            let (mut k, mut i) = (0, 0);
            while i < 8 {
                if bits >> i & 1 != 0 {
                    let mut b = 0;
                    while b < 8 {
                        t[bits][8 * k + b] = (8 * i + b) as u8;
                        b += 1;
                    }
                    k += 1;
                }
                i += 1;
            }
            bits += 1;
        }
        t
    };

    /// The lanes of `v` whose bits are in `bits` (8 of them), in order, at `p`; `p..p + 8` written. Their count.
    ///
    /// # Safety
    /// `p..p + 8` is writable.
    #[inline(always)]
    pub unsafe fn pack(v: V, bits: u64, p: *mut u64) -> usize {
        // Every lane kept: the vector as it is, one store.
        if bits == 0xff {
            p.cast::<[u64; 8]>().write_unaligned(v.to_array());
            return 8;
        }
        let bytes = Simd::<u8, 64>::from_array(core::mem::transmute::<[u64; 8], [u8; 64]>(v.to_array()));
        let ix = Simd::<u8, 64>::from_array(PACK64[bits as usize]);
        p.cast::<[u8; 64]>().write_unaligned(bytes.swizzle_dyn(ix).to_array());
        bits.count_ones() as usize
    }

    /// 8 values at `p`.
    ///
    /// # Safety
    /// `p..p + 8` is readable.
    #[inline(always)]
    pub unsafe fn load(p: *const u64) -> V {
        V::from_array(p.cast::<[u64; 8]>().read_unaligned())
    }

    /// The first place from `i` of `s` (sorted by `k >> sh`) whose `k >> sh` is past `t`: 8 compares a step.
    #[inline(always)]
    pub fn past(s: &[u64], mut i: usize, t: u64, sh: u32) -> usize {
        let ts = V::splat(t);
        let shv = V::splat(u64::from(sh));
        while i + 8 <= s.len() {
            // SAFETY: `i + 8` within `s`.
            let v = unsafe { load(s.as_ptr().add(i)) };
            let le = (v >> shv).simd_le(ts).to_bitmask();
            if le != 0xff {
                return i + le.trailing_ones() as usize;
            }
            i += 8;
        }
        while i < s.len() && s[i] >> sh <= t {
            i += 1;
        }
        i
    }
}

/// The vector kit at 32 bits: 8 or 16 lanes of `u32` (two or four NEON registers).
pub(crate) mod vk32 {
    use rerail_simd::prelude::*;

    pub type V = Simd<u32, 8>;

    /// For each 8 lanes' bits, the bytes of 8 32-bit lanes that move the lanes of the bits to the front.
    pub const PACK32: [[u8; 32]; 256] = {
        let mut t = [[0u8; 32]; 256];
        let mut bits = 0;
        while bits < 256 {
            let (mut k, mut i) = (0, 0);
            while i < 8 {
                if bits >> i & 1 != 0 {
                    let mut b = 0;
                    while b < 4 {
                        t[bits][4 * k + b] = (4 * i + b) as u8;
                        b += 1;
                    }
                    k += 1;
                }
                i += 1;
            }
            bits += 1;
        }
        t
    };

    /// 8 values at `p`.
    ///
    /// # Safety
    /// `p..p + 8` is readable.
    #[inline(always)]
    pub unsafe fn load(p: *const u32) -> V {
        V::from_array(p.cast::<[u32; 8]>().read_unaligned())
    }

    /// The lanes of `v` whose bits are in `bits`, in order, at `p`; `p..p + 8` written. Their count.
    ///
    /// # Safety
    /// `p..p + 8` is writable.
    #[inline(always)]
    pub unsafe fn pack(v: V, bits: u64, p: *mut u32) -> usize {
        if bits == 0xff {
            p.cast::<[u32; 8]>().write_unaligned(v.to_array());
            return 8;
        }
        let bytes = Simd::<u8, 32>::from_array(core::mem::transmute::<[u32; 8], [u8; 32]>(v.to_array()));
        let ix = Simd::<u8, 32>::from_array(PACK32[bits as usize]);
        p.cast::<[u8; 32]>().write_unaligned(bytes.swizzle_dyn(ix).to_array());
        bits.count_ones() as usize
    }

    /// The first place from `i` of `s` (sorted by `k >> sh`) whose `k >> sh` is past `t`: 16 compares a step.
    #[inline(always)]
    pub fn past(s: &[u32], mut i: usize, t: u32, sh: u32) -> usize {
        type W = Simd<u32, 16>;
        let (ts, shv) = (W::splat(t), W::splat(sh));
        while i + 16 <= s.len() {
            // SAFETY: `i + 16` within `s`.
            let v = W::from_array(unsafe { s.as_ptr().add(i).cast::<[u32; 16]>().read_unaligned() });
            let le = (v >> shv).simd_le(ts).to_bitmask();
            if le != 0xffff {
                return i + le.trailing_ones() as usize;
            }
            i += 16;
        }
        while i < s.len() && s[i] >> sh <= t {
            i += 1;
        }
        i
    }

    /// `keys` (sorted) with each key equal to the one before dropped, 8 a step. Into `out`.
    pub fn dedup(keys: &[u32], out: &mut Vec<u32>) {
        out.clear();
        if keys.is_empty() {
            return;
        }
        out.reserve(keys.len() + 8);
        let p = out.as_mut_ptr();
        // SAFETY: `w` at most the keys read; 8 more reserved past them.
        unsafe {
            *p = keys[0];
            let mut w = 1usize;
            let mut i = 1usize;
            while i + 8 <= keys.len() {
                let cur = load(keys.as_ptr().add(i));
                let prev = load(keys.as_ptr().add(i - 1));
                w += pack(cur, cur.simd_ne(prev).to_bitmask(), p.add(w));
                i += 8;
            }
            while i < keys.len() {
                *p.add(w) = keys[i];
                w += usize::from(keys[i] != keys[i - 1]);
                i += 1;
            }
            out.set_len(w);
        }
    }
}

/// `keys` (sorted) with each key equal to the one before dropped, 8 a step: each compared with its neighbour, the
/// kept packed to the front. Into `out`.
pub(crate) fn dedup_simd(keys: &[u64], out: &mut Vec<u64>) {
    use rerail_simd::prelude::*;
    out.clear();
    if keys.is_empty() {
        return;
    }
    out.reserve(keys.len() + 8);
    let p = out.as_mut_ptr();
    // SAFETY: `w` at most the keys read; 8 more reserved past them.
    unsafe {
        *p = keys[0];
        let mut w = 1usize;
        let mut i = 1usize;
        while i + 8 <= keys.len() {
            let cur = vk::load(keys.as_ptr().add(i));
            let prev = vk::load(keys.as_ptr().add(i - 1));
            w += vk::pack(cur, cur.simd_ne(prev).to_bitmask(), p.add(w));
            i += 8;
        }
        while i < keys.len() {
            *p.add(w) = keys[i];
            w += usize::from(keys[i] != keys[i - 1]);
            i += 1;
        }
        out.set_len(w);
    }
}

/// Radix sort with write-combining: each digit's keys gathered in a 64-byte line of their own (in L1) and written
/// out a line at a time, so a pass writes whole lines, not one key into each of `2^d` places. Digits of `d` bits.
pub(crate) fn radix_sort_wc(keys: &mut Vec<u64>, tmp: &mut Vec<u64>, shift: u32, width: u32, d: u32) {
    let n = keys.len();
    if n < 2 || width == 0 {
        return;
    }
    let passes = width.div_ceil(d);
    let d = width.div_ceil(passes);
    let nb = 1usize << d;
    let mask = (nb - 1) as u64;
    let mut hist = vec![0u32; nb * passes as usize];
    for &k in keys.iter() {
        for p in 0..passes as usize {
            // SAFETY: a digit is below `nb`.
            unsafe { *hist.get_unchecked_mut(p * nb + ((k >> (shift + p as u32 * d)) & mask) as usize) += 1 };
        }
    }
    tmp.clear();
    tmp.resize(n, 0);
    let mut lines = vec![[0u64; 8]; nb];
    let mut fill = vec![0u8; nb];
    for p in 0..passes as usize {
        let h = &mut hist[p * nb..(p + 1) * nb];
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
        let out = tmp.as_mut_ptr();
        // SAFETY: a digit's line holds at most 8 keys, written out at its next place when full; its places are
        // its count's, below `n`.
        unsafe {
            for &k in keys.iter() {
                let b = ((k >> sh) & mask) as usize;
                let f = *fill.get_unchecked(b) as usize;
                *lines.get_unchecked_mut(b).get_unchecked_mut(f) = k;
                if f == 7 {
                    let at = *h.get_unchecked(b) as usize;
                    out.add(at).cast::<[u64; 8]>().write_unaligned(*lines.get_unchecked(b));
                    *h.get_unchecked_mut(b) += 8;
                    *fill.get_unchecked_mut(b) = 0;
                } else {
                    *fill.get_unchecked_mut(b) = (f + 1) as u8;
                }
            }
            for b in 0..nb {
                let f = fill[b] as usize;
                core::ptr::copy_nonoverlapping(lines[b].as_ptr(), out.add(h[b] as usize), f);
                fill[b] = 0;
            }
        }
        std::mem::swap(keys, tmp);
    }
}

/// The keyed join's merge at 8 lanes: each key's range found by 8 compares a step (`vk::past`), and each `a` row's
/// pass over its key's `b` range 8 rows a step, `z < x` a compare, the passing rows packed to the front.
fn merge_keyed_simd(a: &[u64], xw: u32, b: &[u64], zw: u32, out: &mut Vec<u64>) {
    use rerail_simd::prelude::*;
    out.clear();
    let (xm, zm) = ((1u64 << xw) - 1, (1u64 << zw) - 1);
    let zmv = vk::V::splat(zm);
    let (mut i, mut j) = (0usize, 0usize);
    while i < a.len() && j < b.len() {
        let (ya, yb) = (a[i] >> xw, b[j] >> zw);
        if ya < yb {
            i = vk::past(a, i, yb - 1, xw);
            continue;
        }
        if ya > yb {
            j = vk::past(b, j, ya - 1, zw);
            continue;
        }
        let i1 = vk::past(a, i, ya, xw);
        let j1 = vk::past(b, j, yb, zw);
        let bz = &b[j..j1];
        out.reserve((i1 - i) * bz.len() + 8);
        for &ak in &a[i..i1] {
            let x = ak & xm;
            let (xs, xv) = (vk::V::splat(x), vk::V::splat(x << zw));
            let mut w = out.len();
            // SAFETY: reserved for every pair of the key's ranges and 8 past them.
            unsafe {
                let p = out.as_mut_ptr();
                let mut q = 0usize;
                while q < bz.len() {
                    let n = (bz.len() - q).min(8);
                    let v = if n == 8 { vk::load(bz.as_ptr().add(q)) } else { vk::V::load_or_default(&bz[q..]) };
                    let z = v & zmv;
                    let bits = z.simd_lt(xs).to_bitmask() & ((1u64 << n) - 1);
                    w += vk::pack(xv | z, bits, p.add(w));
                    q += 8;
                }
                out.set_len(w);
            }
        }
        (i, j) = (i1, j1);
    }
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
    keyed_with(d, false, &mut Scratch::default())
}

/// Buffers a run reuses from the last (as a program's are, kept across its launches): none faulted in again.
#[derive(Default)]
pub(crate) struct Scratch {
    tmp: Vec<u64>,
    a: Vec<u64>,
    b: Vec<u64>,
    out: Vec<u64>,
    kept: Vec<u64>,
}

fn keyed_with(d: &Data, simd: bool, sc: &mut Scratch) -> Keyed {
    let t = std::time::Instant::now;
    let ms = |s: std::time::Instant| s.elapsed().as_secs_f64() * 1e3;
    // Column widths from the relations' maxima.
    let (xw, yw) = (bits(d.a.iter().map(|r| r.0).max().unwrap()), bits(d.a.iter().map(|r| r.1).max().unwrap()));
    let zw = bits(d.b.iter().map(|r| r.1).max().unwrap());
    let Scratch { tmp, a, b, out, kept } = sc;
    let sort = |k: &mut Vec<u64>, t: &mut Vec<u64>, sh: u32, w: u32| if simd { radix_sort_wc(k, t, sh, w, 10) } else { radix_sort(k, t, sh, w) };
    let s = t();
    b.clear();
    b.extend(d.b.iter().map(|&(y, z)| y << zw | z));
    sort(b, tmp, zw, yw);
    let sort_b = ms(s);
    let s = t();
    a.clear();
    a.extend(d.a.iter().map(|&(x, y)| y << xw | x));
    sort(a, tmp, xw, yw);
    let sort_a = ms(s);
    let s = t();
    if simd { merge_keyed_simd(a, xw, b, zw, out) } else { merge_keyed(a, xw, b, zw, out) }
    let merge = ms(s);
    let pairs = out.len();
    let s = t();
    sort(out, tmp, 0, xw + zw);
    let sort_out = ms(s);
    let s = t();
    let rows = if simd {
        dedup_simd(out, kept);
        kept.len()
    } else {
        dedup_sorted(out);
        out.len()
    };
    let dedup = ms(s);
    Keyed { sort_b, sort_a, merge, sort_out, dedup, rows, pairs }
}

/// A relation's two columns as stored: each value in the narrowest integer that holds it (here 32 bits).
pub(crate) struct Cols {
    pub c0: Vec<u32>,
    pub c1: Vec<u32>,
}

fn cols(r: &[(u64, u64)]) -> Cols {
    Cols { c0: r.iter().map(|x| x.0 as u32).collect(), c1: r.iter().map(|x| x.1 as u32).collect() }
}

/// `keys` with `vals` (a row each place) sorted by `keys`' low `width` bits: a stable LSD radix sort, digits of at
/// most 11 bits, every pass's histogram counted in one read, a pass whose digit is the same for every key skipped.
pub(crate) fn radix_sort_kv(keys: &mut Vec<u32>, vals: &mut Vec<u32>, tk: &mut Vec<u32>, tv: &mut Vec<u32>, width: u32) {
    let n = keys.len();
    if n < 2 || width == 0 {
        return;
    }
    let passes = width.div_ceil(11);
    let d = width.div_ceil(passes);
    let mask = (1u32 << d) - 1;
    let mut hist = vec![[0u32; 2048]; passes as usize];
    for &k in keys.iter() {
        for (p, h) in hist.iter_mut().enumerate() {
            h[((k >> (p as u32 * d)) & mask) as usize] += 1;
        }
    }
    tk.clear();
    tk.resize(n, 0);
    tv.clear();
    tv.resize(n, 0);
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
        let sh = p as u32 * d;
        for r in 0..n {
            // SAFETY: `r < n`; `h[b]` counts the keys before this one of its digit: below `n`.
            unsafe {
                let k = *keys.get_unchecked(r);
                let b = ((k >> sh) & mask) as usize;
                let at = *h.get_unchecked(b) as usize;
                *tk.get_unchecked_mut(at) = k;
                *tv.get_unchecked_mut(at) = *vals.get_unchecked(r);
                *h.get_unchecked_mut(b) += 1;
            }
        }
        std::mem::swap(keys, tk);
        std::mem::swap(vals, tv);
    }
}

/// The keyed join's pairs `x << zw | z` (40 bits: a 64-bit key) from `a`'s columns `(y, x)` and `b`'s `(y, z)`, each
/// sorted by `y`.
fn merge_cols(ay: &[u32], ax: &[u32], by: &[u32], bz: &[u32], zw: u32, out: &mut Vec<u64>) {
    out.clear();
    let (mut i, mut j) = (0usize, 0usize);
    while i < ay.len() && j < by.len() {
        let (ya, yb) = (ay[i], by[j]);
        if ya < yb {
            i += 1;
            continue;
        }
        if ya > yb {
            j += 1;
            continue;
        }
        let i1 = i + ay[i..].iter().take_while(|&&v| v == ya).count();
        let j1 = j + by[j..].iter().take_while(|&&v| v == yb).count();
        let zs = &bz[j..j1];
        out.reserve((i1 - i) * zs.len());
        for &x in &ax[i..i1] {
            let at = out.len();
            // SAFETY: reserved for every pair of the key's ranges; the length counts those written.
            unsafe {
                let p = out.as_mut_ptr().add(at);
                let mut w = 0usize;
                for &z in zs {
                    *p.add(w) = u64::from(x) << zw | u64::from(z);
                    w += usize::from(z < x);
                }
                out.set_len(at + w);
            }
        }
        (i, j) = (i1, j1);
    }
}

/// `keys` (sorted) without repeats, written as the head's two 32-bit columns `x` and `z`: one pass, no branch on
/// the data.
fn dedup_split(keys: &[u64], zw: u32, x: &mut Vec<u32>, z: &mut Vec<u32>) {
    let n = keys.len();
    x.clear();
    z.clear();
    x.reserve(n);
    z.reserve(n);
    let zm = (1u64 << zw) - 1;
    let (px, pz) = (x.as_mut_ptr(), z.as_mut_ptr());
    let mut w = 0usize;
    let mut prev = u64::MAX;
    // SAFETY: `w` at most the keys read, below `n`, reserved.
    unsafe {
        for &k in keys {
            *px.add(w) = (k >> zw) as u32;
            *pz.add(w) = (k & zm) as u32;
            w += usize::from(k != prev);
            prev = k;
        }
        x.set_len(w);
        z.set_len(w);
    }
}

/// Buffers of the 32-bit keyed join, reused run to run.
#[derive(Default)]
pub(crate) struct Scratch32 {
    tk: Vec<u32>,
    tv: Vec<u32>,
    ay: Vec<u32>,
    ax: Vec<u32>,
    by: Vec<u32>,
    bz: Vec<u32>,
    out: Vec<u64>,
    tmp: Vec<u64>,
    hx: Vec<u32>,
    hz: Vec<u32>,
}

/// The keyed join with every stored value in 32 bits: `a` (`x`, `y`) and `b` (`y`, `z`) as 32-bit columns, each
/// sorted by `y` with its other column carried; the merge reads the columns; the pairs (40 bits) a 64-bit key sorted
/// and made distinct; the head written as 32-bit columns `x`, `z` (its dedup's time).
fn keyed32(a: &Cols, b: &Cols, sc: &mut Scratch32) -> Keyed {
    let t = std::time::Instant::now;
    let ms = |s: std::time::Instant| s.elapsed().as_secs_f64() * 1e3;
    let yw = bits(u64::from(*a.c1.iter().max().unwrap()));
    let (xw, zw) = (bits(u64::from(*a.c0.iter().max().unwrap())), bits(u64::from(*b.c1.iter().max().unwrap())));
    let Scratch32 { tk, tv, ay, ax, by, bz, out, tmp, hx, hz } = sc;
    let s = t();
    by.clear();
    by.extend_from_slice(&b.c0);
    bz.clear();
    bz.extend_from_slice(&b.c1);
    radix_sort_kv(by, bz, tk, tv, yw);
    let sort_b = ms(s);
    let s = t();
    ay.clear();
    ay.extend_from_slice(&a.c1);
    ax.clear();
    ax.extend_from_slice(&a.c0);
    radix_sort_kv(ay, ax, tk, tv, yw);
    let sort_a = ms(s);
    let s = t();
    merge_cols(ay, ax, by, bz, zw, out);
    let merge = ms(s);
    let pairs = out.len();
    let s = t();
    radix_sort(out, tmp, 0, xw + zw);
    let sort_out = ms(s);
    let s = t();
    dedup_split(out, zw, hx, hz);
    let dedup = ms(s);
    Keyed { sort_b, sort_a, merge, sort_out, dedup, rows: hx.len(), pairs }
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
    /// Rows into the minus (each round's new rows, distinct), and run rows it walked or searched past.
    pub minus_in: usize,
    pub runs_seen: usize,
}

fn closure(d: &Data) -> Closure {
    closure_with(d, false, false)
}

/// `runs`: h0 as sorted runs (`Runs`), new rows searched for by galloping; else one sorted h0 merged whole each round.
/// `simd`: the vector kit's sort, scans, dedup and branchless merges.
fn closure_with(d: &Data, runs: bool, simd: bool) -> Closure {
    closure_held::<u64>(d, if runs { Held::Runs } else { Held::Whole }, simd)
}

/// How a closure keeps h0's rows for the minus.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Held {
    /// One sorted vector, merged whole each round.
    Whole,
    /// Sorted runs (`Runs`).
    Runs,
    /// A bit a possible row, where the rows' packed width is small (its columns' statistics say so): a row's test and
    /// insert are one bit; h0's sorted rows are the bits' places, read once at the end.
    Dense,
}

/// `K`: the integer each row is stored in (`u32` where its packed width fits, as here: 18 bits).
fn closure_held<K: Key>(d: &Data, how: Held, simd: bool) -> Closure {
    let runs = how == Held::Runs;
    let t = std::time::Instant::now;
    let ms = |s: std::time::Instant| s.elapsed().as_secs_f64() * 1e3;
    let w = bits(d.e.iter().map(|r| r.0.max(r.1)).max().unwrap());
    let m = (1u64 << w) - 1;
    let mut tmp = Vec::new();
    let mut c = Closure { sort_e: 0.0, sort_delta: 0.0, merge: 0.0, sort_new: 0.0, minus: 0.0, union: 0.0, rounds: 0, rows: 0, minus_in: 0, runs_seen: 0 };
    // `e` by its row (`src << w | dst`): its first column leads, so it is sorted by the join key too.
    let s = t();
    let mut e: Vec<K> = d.e.iter().map(|&(s, t)| K::of(s << w | t)).collect();
    radix_sort(&mut e, &mut tmp, 0, 2 * w);
    dedup_sorted(&mut e);
    c.sort_e = ms(s);
    // The seed: h0 = e. Rows `x << w | y`, sorted.
    // The write-combined sort lost to the plain one (46837f2e49): one sort for both.
    let sort = |k: &mut Vec<K>, t: &mut Vec<K>, sh: u32, w: u32| radix_sort(k, t, sh, w);
    let mut h0 = e.clone();
    let mut held = Runs { runs: vec![e.clone()] };
    let mut delta = h0.clone();
    let mut kept = Vec::new();
    let mut dense = if how == Held::Dense { vec![0u64; (1usize << (2 * w)).div_ceil(64)] } else { Vec::new() };
    for &k in &e {
        let k = k.u();
        if how == Held::Dense {
            dense[(k >> 6) as usize] |= 1u64 << (k & 63);
        }
    }
    let (mut by_y, mut new, mut fresh, mut merged): (Vec<K>, Vec<K>, Vec<K>, Vec<K>) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    while !delta.is_empty() {
        c.rounds += 1;
        // The delta by its join column: `y << w | x`.
        let s = t();
        by_y.clear();
        by_y.extend(delta.iter().map(|&k| K::of((k.u() & m) << w | k.u() >> w)));
        sort(&mut by_y, &mut tmp, w, w);
        c.sort_delta += ms(s);
        // Merged with e: each y's delta rows crossed with its edges, rows `x << w | z`.
        let s = t();
        new.clear();
        let (mut i, mut j) = (0usize, 0usize);
        while i < by_y.len() && j < e.len() {
            let (ya, yb) = (by_y[i].u() >> w, e[j].u() >> w);
            if ya < yb {
                i += 1;
                continue;
            }
            if ya > yb {
                j += 1;
                continue;
            }
            let (i1, j1) = if simd {
                (K::past(&by_y, i, ya, w), K::past(&e, j, yb, w))
            } else {
                (i + by_y[i..].iter().take_while(|&&k| k.u() >> w == ya).count(), j + e[j..].iter().take_while(|&&k| k.u() >> w == yb).count())
            };
            for &dk in &by_y[i..i1] {
                let x = dk.u() & m;
                new.extend(e[j..j1].iter().map(|&ek| K::of(x << w | (ek.u() & m))));
            }
            (i, j) = (i1, j1);
        }
        c.merge += ms(s);
        let s = t();
        // A dense h0 drops repeats as it takes them, and the next round sorts its delta by y: no sort here.
        if how != Held::Dense {
        sort(&mut new, &mut tmp, 0, 2 * w);
        if simd {
            K::dedup_simd(&new, &mut kept);
            std::mem::swap(&mut new, &mut kept);
        } else {
            dedup_sorted(&mut new);
        }
        }
        c.sort_new += ms(s);
        c.minus_in += new.len();
        if how == Held::Dense {
            let s = t();
            let mut w = 0usize;
            for r in 0..new.len() {
                let k = new[r];
                let (word, bit) = ((k.u() >> 6) as usize, 1u64 << (k.u() & 63));
                let had = dense[word] & bit != 0;
                dense[word] |= bit;
                new[w] = k;
                w += usize::from(!had);
            }
            new.truncate(w);
            c.minus += ms(s);
            std::mem::swap(&mut delta, &mut new);
            continue;
        }
        if runs {
            c.runs_seen += held.runs.iter().map(Vec::len).sum::<usize>();
            let s = t();
            held.minus(&mut new);
            c.minus += ms(s);
            let s = t();
            held.add(new.clone(), simd);
            c.union += ms(s);
            std::mem::swap(&mut delta, &mut new);
            continue;
        }
        let s = t();
        sorted_minus(&new, &h0, &mut fresh);
        c.minus += ms(s);
        let s = t();
        if simd { union_branchless(&h0, &fresh, &mut merged) } else { sorted_union(&h0, &fresh, &mut merged) }
        std::mem::swap(&mut h0, &mut merged);
        c.union += ms(s);
        std::mem::swap(&mut delta, &mut fresh);
    }
    c.rows = match how {
        Held::Runs => held.len(),
        Held::Whole => h0.len(),
        Held::Dense => {
            // h0's rows in order: each set bit's place.
            let s = t();
            let mut rows = Vec::with_capacity(c.minus_in);
            for (i, &word) in dense.iter().enumerate() {
                let mut b = word;
                while b != 0 {
                    rows.push(K::of((i as u64) << 6 | u64::from(b.trailing_zeros())));
                    b &= b - 1;
                }
            }
            c.union += ms(s);
            rows.len()
        }
    };
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
        assert_eq!(keyed_with(&d, true, &mut Scratch::default()).rows, want.len());
        assert_eq!(keyed32(&cols(&d.a), &cols(&d.b), &mut Scratch32::default()).rows, want.len());
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
        for (runs, simd) in [(false, false), (true, false), (false, true), (true, true)] {
            assert_eq!(closure_with(&d, runs, simd).rows, h.len(), "runs {runs}, simd {simd}");
        }
        assert_eq!(closure_held::<u64>(&d, Held::Dense, false).rows, h.len());
        for how in [Held::Whole, Held::Runs, Held::Dense] {
            for simd in [false, true] {
                assert_eq!(closure_held::<u32>(&d, how, simd).rows, h.len(), "u32 {how:?}, simd {simd}");
            }
        }
    }

    /// The spike's times, the best of 20 runs each, scalar and vector, part by part (a measure: `--ignored`, at
    /// opt-level 3, pinned to one core).
    #[test]
    #[ignore]
    fn the_spike_timed() {
        let d = data();
        let total_k = |k: &Keyed| k.sort_a + k.merge + k.sort_out + k.dedup;
        for simd in [false, true] {
            let mut best: Option<Keyed> = None;
            let mut sc = Scratch::default();
            for _ in 0..20 {
                let k = keyed_with(&d, simd, &mut sc);
                if best.as_ref().is_none_or(|b| total_k(&k) < total_k(b)) {
                    best = Some(k);
                }
            }
            let k = best.unwrap();
            eprintln!(
                "keyed {}: {} pairs, {} rows; sort b {:.2} ms (once), sort a {:.2}, merge {:.2}, sort out {:.2}, dedup {:.2}: {:.2} ms",
                if simd { "simd  " } else { "scalar" }, k.pairs, k.rows, k.sort_b, k.sort_a, k.merge, k.sort_out, k.dedup, total_k(&k)
            );
        }
        let total_c = |c: &Closure| c.sort_delta + c.merge + c.sort_new + c.minus + c.union;
        for (runs, simd) in [(false, false), (true, false), (true, true)] {
            let mut best: Option<Closure> = None;
            for _ in 0..20 {
                let c = closure_with(&d, runs, simd);
                if best.as_ref().is_none_or(|b| total_c(&c) < total_c(b)) {
                    best = Some(c);
                }
            }
            let c = best.unwrap();
            eprintln!(
                "closure {} {}: {} rounds, {} rows; sort delta {:.2}, merge {:.2}, sort new {:.2}, minus {:.2}, union {:.2}: {:.2} ms; {} rows into the minus, {} run rows",
                if runs { "runs " } else { "whole" }, if simd { "simd  " } else { "scalar" }, c.rounds, c.rows, c.sort_delta, c.merge, c.sort_new, c.minus, c.union, total_c(&c), c.minus_in, c.runs_seen
            );
        }
        let mut best: Option<Closure> = None;
        for _ in 0..20 {
            let c = closure_held::<u64>(&d, Held::Dense, false);
            if best.as_ref().is_none_or(|b| total_c(&c) < total_c(b)) {
                best = Some(c);
            }
        }
        let c = best.unwrap();
        eprintln!(
            "closure dense scalar: {} rounds, {} rows; sort delta {:.2}, merge {:.2}, sort new {:.2}, minus {:.2}, rows read out {:.2}: {:.2} ms",
            c.rounds, c.rows, c.sort_delta, c.merge, c.sort_new, c.minus, c.union, total_c(&c)
        );
        // Every stored value in 32 bits.
        let (a, b) = (cols(&d.a), cols(&d.b));
        let mut best: Option<Keyed> = None;
        let mut sc = Scratch32::default();
        for _ in 0..20 {
            let k = keyed32(&a, &b, &mut sc);
            if best.as_ref().is_none_or(|b| total_k(&k) < total_k(b)) {
                best = Some(k);
            }
        }
        let k = best.unwrap();
        eprintln!(
            "keyed 32-bit columns: {} pairs, {} rows; sort b {:.2} ms (once), sort a {:.2}, merge {:.2}, sort out {:.2}, dedup and write {:.2}: {:.2} ms",
            k.pairs, k.rows, k.sort_b, k.sort_a, k.merge, k.sort_out, k.dedup, total_k(&k)
        );
        for (how, simd) in [(Held::Whole, false), (Held::Runs, false), (Held::Runs, true), (Held::Dense, false)] {
            let mut best: Option<Closure> = None;
            for _ in 0..20 {
                let c = closure_held::<u32>(&d, how, simd);
                if best.as_ref().is_none_or(|b| total_c(&c) < total_c(b)) {
                    best = Some(c);
                }
            }
            let c = best.unwrap();
            eprintln!(
                "closure u32 {how:?} {}: {} rows; sort delta {:.2}, merge {:.2}, sort new {:.2}, minus {:.2}, union {:.2}: {:.2} ms",
                if simd { "simd  " } else { "scalar" }, c.rows, c.sort_delta, c.merge, c.sort_new, c.minus, c.union, total_c(&c)
            );
        }
        // The radix sorts alone, on the keyed join's output (its largest): 11-bit digits, then write-combined lines.
        let mut out = Vec::new();
        {
            let (xw, zw) = (20, 20);
            let mut a: Vec<u64> = d.a.iter().map(|&(x, y)| y << xw | x).collect();
            let mut b: Vec<u64> = d.b.iter().map(|&(y, z)| y << zw | z).collect();
            let mut tmp = Vec::new();
            radix_sort(&mut a, &mut tmp, xw, 15);
            radix_sort(&mut b, &mut tmp, zw, 15);
            merge_keyed(&a, xw, &b, zw, &mut out);
        }
        let time = |f: &dyn Fn(&mut Vec<u64>, &mut Vec<u64>)| {
            let mut best = f64::MAX;
            for _ in 0..20 {
                let mut k = out.clone();
                let mut t = Vec::new();
                let s = std::time::Instant::now();
                f(&mut k, &mut t);
                best = best.min(s.elapsed().as_secs_f64() * 1e3);
            }
            best
        };
        eprintln!(
            "radix of {} keys, 40 bits: 11-bit digits {:.2} ms; write-combined 8-bit {:.2}, 10-bit {:.2}, 11-bit {:.2}",
            out.len(),
            time(&|k, t| radix_sort(k, t, 0, 40)),
            time(&|k, t| radix_sort_wc(k, t, 0, 40, 8)),
            time(&|k, t| radix_sort_wc(k, t, 0, 40, 10)),
            time(&|k, t| radix_sort_wc(k, t, 0, 40, 11)),
        );
    }
}
