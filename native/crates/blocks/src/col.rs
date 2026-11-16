//! Frozen integer columns: written once, at freeze, in the fewest bits their values need (MLDB's
//! `RawMappedIntTable`: one small descriptor, the encoding chosen by exact size).
//!
//! - `CONST`: every value is `base`. No bytes.
//! - `PACK`: `v - base` in `bits` bits, packed into 64-bit words (frame of reference).
//! - `RUNS`: runs of equal values: each run's value (`v - base`, `bits` bits) and its first position (`rbits`).
//!   A sorted column with repeats, as a canonical order's leading column is.
//! - `LINE`: `v - (base + i * slope / 2^16)`, less `rmin`, in `bits` bits (MLDB's `Predictor`): a column that grows
//!   with its position, as ids in canonical order do.
//!
//! Reads are arithmetic on the words: the same on a GPU, where a column is read for its bandwidth and the extra
//! instructions to decode it cost nothing.

use crate::block::Block;
use crate::refs::Span;
use crate::structure;

pub const CONST: u32 = 0;
pub const PACK: u32 = 1;
pub const RUNS: u32 = 2;
pub const LINE: u32 = 3;

structure! {
    pub struct IntCol {
        pub tag: u32,
        /// Bits a value (or a run's value, or a residual) takes.
        pub bits: u32,
        pub n: u32,
        /// `RUNS`: bits a run's first position takes, and the run count.
        pub rbits: u32,
        pub nruns: u32,
        pub pad: u32,
        pub base: i64,
        /// `LINE`: the slope, in 1/65,536ths a row, and the residuals' minimum.
        pub slope: i64,
        pub rmin: i64,
        /// Packed values (one spare word at the end, so a read never branches on a word boundary).
        pub data: Span<u64>,
        /// `RUNS`: packed first positions.
        pub runs: Span<u64>,
    }
}

fn bits_for(range: u64) -> u32 {
    64 - range.leading_zeros()
}

#[inline]
fn read(words: &[u64], i: usize, bits: u32) -> u64 {
    if bits == 0 {
        return 0;
    }
    let bit = i * bits as usize;
    let (w, sh) = (bit >> 6, bit & 63);
    let two = words[w] as u128 | (words[w + 1] as u128) << 64;
    let mask = if bits == 64 { u64::MAX } else { (1u64 << bits) - 1 };
    (two >> sh) as u64 & mask
}

/// Words `n` values of `bits` bits take, and the spare one.
fn words(n: usize, bits: u32) -> usize {
    (n * bits as usize).div_ceil(64) + 1
}

/// Packs `vals` (each under `2^bits`) into `out`, which is zero and `words(vals.len(), bits)` long. 64 values make
/// `bits` whole words, so a group of 64 is packed by a loop whose shifts and word boundaries are constants for each
/// width (`pack_fixed`, one per width); the tail goes value by value.
pub fn pack_into(out: &mut [u64], vals: &[u64], bits: u32) {
    if bits == 0 {
        return;
    }
    macro_rules! widths {
        ($($b:literal)*) => {
            match bits { $($b => pack_fixed::<$b>(out, vals),)* _ => unreachable!("{bits} bits") }
        };
    }
    widths!(1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31 32
            33 34 35 36 37 38 39 40 41 42 43 44 45 46 47 48 49 50 51 52 53 54 55 56 57 58 59 60 61 62 63 64);
}

#[inline(never)]
fn pack_fixed<const B: usize>(out: &mut [u64], vals: &[u64]) {
    let groups = vals.len() / 64;
    for g in 0..groups {
        let v: &[u64; 64] = vals[g * 64..g * 64 + 64].try_into().unwrap();
        let o: &mut [u64] = &mut out[g * B..g * B + B + 1];
        for (j, &x) in v.iter().enumerate() {
            let bit = j * B;
            let (w, sh) = (bit / 64, bit % 64);
            o[w] |= x << sh;
            if sh + B > 64 {
                o[w + 1] |= x >> (64 - sh);
            }
        }
    }
    for (i, &x) in vals.iter().enumerate().skip(groups * 64) {
        let bit = i * B;
        let (w, sh) = (bit / 64, bit % 64);
        out[w] |= x << sh;
        if sh + B > 64 {
            out[w + 1] |= x >> (64 - sh);
        }
    }
}

/// A column's encoding, chosen by exact size before anything is packed: so a block can be made exactly the size
/// of what it will hold, and each column packed straight into it.
#[derive(Clone, Copy, Debug)]
pub struct Choice {
    pub tag: u32,
    pub bits: u32,
    pub rbits: u32,
    pub nruns: u32,
    pub base: i64,
    pub slope: i64,
    pub rmin: i64,
    pub n: usize,
    pub data_words: usize,
    pub runs_words: usize,
}

impl Choice {
    /// The bytes it takes in a block, besides its descriptor.
    pub fn bytes(&self) -> usize {
        if self.tag == CONST { 0 } else { (self.data_words + self.runs_words) * 8 }
    }
}

/// A column chosen and packed, not yet placed: its descriptor's fields and its words.
pub struct Encoded {
    pub tag: u32,
    pub bits: u32,
    pub rbits: u32,
    pub nruns: u32,
    pub base: i64,
    pub slope: i64,
    pub rmin: i64,
    pub n: usize,
    pub data: Vec<u64>,
    pub runs: Vec<u64>,
}

impl Encoded {
    /// The bytes it takes in a block, besides its descriptor.
    pub fn bytes(&self) -> usize {
        if self.tag == CONST { 0 } else { (self.data.len() + self.runs.len()) * 8 }
    }
}

#[inline]
fn line(base: i64, slope: i64, i: usize) -> i64 {
    base + ((i as i128 * slope as i128) >> 16) as i64
}

/// The line's residuals over `v` (`v[i] - (v[0] + i * slope / 2^16)`): the words they pack in, their bits and
/// their minimum, or `usize::MAX` words when the line cannot beat `bits` a value. A sample (every 64th row) is
/// tried first: its residuals' range is within the full one, so when the sample already needs `bits`, the full
/// pass is skipped. The pass runs in `i64` when `slope * n` cannot overflow it (vectorized), else in `i128`.
fn line_fit(v: &[i64], slope: i64, bits: u32) -> (usize, u32, i64) {
    let n = v.len();
    let no = (usize::MAX, 64, 0);
    let v0 = v[0] as i128;
    let resid = |i: usize| v[i] as i128 - (v0 + ((i as i128 * slope as i128) >> 16));
    let (slo, shi) = (0..n).step_by(64).map(resid).fold((i128::MAX, i128::MIN), |(a, b), r| (a.min(r), b.max(r)));
    if shi - slo > u64::MAX as i128 || bits_for((shi - slo) as u64) >= bits {
        return no;
    }
    let small = (slope as i128).abs() * n as i128 <= (1i128 << 62) && v.iter().all(|&x| x.unsigned_abs() < 1 << 61);
    let (rlo, rhi) = if small {
        let (v0, slope) = (v[0], slope);
        let (mut lo, mut hi, mut t) = (i64::MAX, i64::MIN, 0i64);
        for &x in v {
            let r = x - (v0 + (t >> 16));
            lo = lo.min(r);
            hi = hi.max(r);
            t += slope;
        }
        (lo as i128, hi as i128)
    } else {
        (0..n).map(resid).fold((i128::MAX, i128::MIN), |(a, b), r| (a.min(r), b.max(r)))
    };
    let range = rhi - rlo;
    if range > u64::MAX as i128 || rlo < i64::MIN as i128 {
        return no;
    }
    let lbits = bits_for(range as u64);
    (words(n, lbits), lbits, rlo as i64)
}

/// The smallest encoding of `v`: one pass for the range and the runs, one for the line's residuals (when a sample
/// says the line may win).
pub fn choose(v: &[i64]) -> Choice {
    let n = v.len();
    let none = Choice { tag: CONST, bits: 0, rbits: 0, nruns: 0, base: 0, slope: 0, rmin: 0, n, data_words: 1, runs_words: 0 };
    if n == 0 {
        return none;
    }
    // The range and the run count in one branch-free pass (vectorized).
    let (lo, hi) = v.iter().fold((v[0], v[0]), |(a, b), &x| (a.min(x), b.max(x)));
    let nruns = 1 + v[1..].iter().zip(v).map(|(a, b)| (a != b) as usize).sum::<usize>();
    if lo == hi {
        return Choice { base: lo, ..none };
    }
    let bits = bits_for((hi as i128 - lo as i128) as u64);
    let pack_words = words(n, bits);
    let rbits = bits_for(n as u64);
    let runs = (words(nruns, bits), words(nruns, rbits));
    let slope = if n > 1 { (((v[n - 1] as i128 - v[0] as i128) << 16) / (n as i128 - 1)) as i64 } else { 0 };
    let (line_words, lbits, rlo) = line_fit(v, slope, bits);
    let best = pack_words.min(runs.0 + runs.1).min(line_words);
    if best == runs.0 + runs.1 {
        return Choice { tag: RUNS, bits, rbits, nruns: nruns as u32, base: lo, slope: 0, rmin: 0, n, data_words: runs.0, runs_words: runs.1 };
    }
    if best == line_words && line_words < pack_words {
        return Choice { tag: LINE, bits: lbits, rbits: 0, nruns: 0, base: v[0], slope, rmin: rlo, n, data_words: line_words, runs_words: 0 };
    }
    Choice { tag: PACK, bits, rbits: 0, nruns: 0, base: lo, slope: 0, rmin: 0, n, data_words: pack_words, runs_words: 0 }
}

/// Packs `v` as `c` chose into `data` and `runs` (zero, `c.data_words` and `c.runs_words` long). `tmp` is room
/// for `v.len()` values.
pub fn write(c: &Choice, v: &[i64], data: &mut [u64], runs: &mut [u64], tmp: &mut [u64]) {
    let n = v.len();
    match c.tag {
        CONST => {}
        PACK => {
            for (t, &x) in tmp.iter_mut().zip(v) {
                *t = x.wrapping_sub(c.base) as u64;
            }
            pack_into(data, &tmp[..n], c.bits);
        }
        LINE => {
            if (c.slope as i128).abs() * n as i128 <= (1i128 << 62) && v.iter().all(|&x| x.unsigned_abs() < 1 << 61) {
                // As `line_fit`: no overflow in i64.
                let mut acc = 0i64;
                for (t, &x) in tmp.iter_mut().zip(v) {
                    *t = (x - (c.base + (acc >> 16)) - c.rmin) as u64;
                    acc += c.slope;
                }
            } else {
                let mut acc = 0i128;
                for (t, &x) in tmp.iter_mut().zip(v) {
                    *t = (x as i128 - (c.base as i128 + (acc >> 16)) - c.rmin as i128) as u64;
                    acc += c.slope as i128;
                }
            }
            pack_into(data, &tmp[..n], c.bits);
        }
        _ => {
            let k = c.nruns as usize;
            let (vals, starts) = tmp[..2 * k].split_at_mut(k);
            let mut r = 0;
            for i in 0..n {
                if i == 0 || v[i] != v[i - 1] {
                    vals[r] = v[i].wrapping_sub(c.base) as u64;
                    starts[r] = i as u64;
                    r += 1;
                }
            }
            pack_into(data, vals, c.bits);
            pack_into(runs, starts, c.rbits);
        }
    }
}

/// The smallest of the encodings for `v`, packed.
pub fn encode(v: &[i64]) -> Encoded {
    let c = choose(v);
    let (mut data, mut runs) = (vec![0u64; c.data_words], vec![0u64; c.runs_words]);
    let mut tmp = vec![0u64; v.len().max(2 * c.nruns as usize)];
    write(&c, v, &mut data, &mut runs, &mut tmp);
    Encoded { tag: c.tag, bits: c.bits, rbits: c.rbits, nruns: c.nruns, base: c.base, slope: c.slope, rmin: c.rmin, n: c.n, data, runs }
}

impl IntCol {
    /// Writes `e` into `block` at `this`.
    ///
    /// # Safety
    /// `this` is zeroed memory in `block`, which is open.
    pub unsafe fn place(this: *mut IntCol, block: &Block, e: &Encoded) {
        unsafe {
            (*this).tag = e.tag;
            (*this).bits = e.bits;
            (*this).n = e.n as u32;
            (*this).rbits = e.rbits;
            (*this).nruns = e.nruns;
            (*this).base = e.base;
            (*this).slope = e.slope;
            (*this).rmin = e.rmin;
            if e.tag != CONST {
                let d: *mut u64 = block.alloc_t(e.data.len());
                std::ptr::copy_nonoverlapping(e.data.as_ptr(), d, e.data.len());
                Span::set(&raw mut (*this).data, d, e.data.len());
            }
            if !e.runs.is_empty() {
                let r: *mut u64 = block.alloc_t(e.runs.len());
                std::ptr::copy_nonoverlapping(e.runs.as_ptr(), r, e.runs.len());
                Span::set(&raw mut (*this).runs, r, e.runs.len());
            }
        }
    }

    /// Writes `v`, encoded as `c` chose, into `block` at `this`: the words allocated there and packed in place.
    /// `tmp` is room for `v.len()` values.
    ///
    /// # Safety
    /// `this` is zeroed memory in `block`, which is open.
    pub unsafe fn place_chosen(this: *mut IntCol, block: &Block, c: &Choice, v: &[i64], tmp: &mut [u64]) {
        unsafe {
            (*this).tag = c.tag;
            (*this).bits = c.bits;
            (*this).n = c.n as u32;
            (*this).rbits = c.rbits;
            (*this).nruns = c.nruns;
            (*this).base = c.base;
            (*this).slope = c.slope;
            (*this).rmin = c.rmin;
            if c.tag == CONST {
                return;
            }
            let d: *mut u64 = block.alloc_t(c.data_words);
            let r: *mut u64 = if c.runs_words > 0 { block.alloc_t(c.runs_words) } else { std::ptr::NonNull::dangling().as_ptr() };
            let data = std::slice::from_raw_parts_mut(d, c.data_words);
            let runs = std::slice::from_raw_parts_mut(r, c.runs_words);
            write(c, v, data, runs, tmp);
            Span::set(&raw mut (*this).data, d, c.data_words);
            if c.runs_words > 0 {
                Span::set(&raw mut (*this).runs, r, c.runs_words);
            }
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.n as usize
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    #[inline]
    fn run_start(&self, k: usize) -> usize {
        read(self.runs.as_slice(), k, self.rbits) as usize
    }

    /// The run holding position `i`.
    #[inline]
    fn run_of(&self, i: usize) -> usize {
        let (mut lo, mut hi) = (0usize, self.nruns as usize);
        while hi - lo > 1 {
            let mid = (lo + hi) / 2;
            if self.run_start(mid) <= i { lo = mid } else { hi = mid }
        }
        lo
    }

    #[inline]
    pub fn get(&self, i: usize) -> i64 {
        match self.tag {
            CONST => self.base,
            PACK => self.base.wrapping_add(read(self.data.as_slice(), i, self.bits) as i64),
            RUNS => self.base.wrapping_add(read(self.data.as_slice(), self.run_of(i), self.bits) as i64),
            _ => (line(self.base, self.slope, i) as i128 + self.rmin as i128 + read(self.data.as_slice(), i, self.bits) as i128) as i64,
        }
    }

    /// Every value in order, decoded as it goes (a run's value once per run, not a search per row).
    pub fn for_each(&self, mut f: impl FnMut(i64)) {
        let n = self.len();
        match self.tag {
            CONST => (0..n).for_each(|_| f(self.base)),
            RUNS => {
                let d = self.data.as_slice();
                for k in 0..self.nruns as usize {
                    let end = if k + 1 < self.nruns as usize { self.run_start(k + 1) } else { n };
                    let v = self.base.wrapping_add(read(d, k, self.bits) as i64);
                    for _ in self.run_start(k)..end {
                        f(v);
                    }
                }
            }
            _ => (0..n).for_each(|i| f(self.get(i))),
        }
    }

    /// The first position in `lo..hi` whose value is not less than `key`, the column sorted over that range.
    pub fn lower_bound(&self, mut lo: usize, mut hi: usize, key: i64) -> usize {
        if self.tag == RUNS {
            // A run at a time: the search reads the runs, not the rows.
            let d = self.data.as_slice();
            let (mut a, mut b) = (self.run_of(lo), self.run_of(hi.max(1) - 1) + 1);
            while a < b {
                let m = (a + b) / 2;
                if self.base.wrapping_add(read(d, m, self.bits) as i64) < key { a = m + 1 } else { b = m }
            }
            let at = if a >= self.nruns as usize { self.len() } else { self.run_start(a) };
            return at.clamp(lo, hi);
        }
        while lo < hi {
            let m = (lo + hi) / 2;
            if self.get(m) < key { lo = m + 1 } else { hi = m }
        }
        lo
    }

    pub fn bytes(&self) -> usize {
        (self.data.len() + self.runs.len()) * 8
    }

    pub fn tag_name(&self) -> &'static str {
        ["const", "pack", "runs", "line"][self.tag as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packing_reads_back_at_every_width() {
        for bits in 1..=64u32 {
            let mask = if bits == 64 { u64::MAX } else { (1u64 << bits) - 1 };
            let mut x = 0x2545_F491_4F6C_DD1Du64 ^ bits as u64;
            for n in [0usize, 1, 63, 64, 65, 200] {
                let vals: Vec<u64> = (0..n)
                    .map(|_| {
                        x ^= x << 13;
                        x ^= x >> 7;
                        x ^= x << 17;
                        x & mask
                    })
                    .collect();
                let mut w = vec![0u64; words(n, bits)];
                pack_into(&mut w, &vals, bits);
                for (i, &v) in vals.iter().enumerate() {
                    assert_eq!(read(&w, i, bits), v, "{bits} bits, value {i} of {n}");
                }
            }
        }
    }

    #[test]
    fn each_encoding_reads_back() {
        let cases: Vec<Vec<i64>> = vec![
            vec![5; 100],
            (0..1000).map(|i| (i * 7919) % 1000 - 300).collect(),
            (0..1000).map(|i| i / 37).collect(),
            (0..1000).map(|i| 1_000_000 + i * 3 + i % 5).collect(),
            vec![i64::MIN, i64::MAX, 0],
        ];
        let tags: Vec<u32> = cases.iter().map(|v| encode(v).tag).collect();
        assert_eq!(&tags[..4], &[CONST, PACK, RUNS, LINE]);
        for v in &cases {
            let e = encode(v);
            let block = crate::block::Block::anon(crate::block::kind::FACTS, 1 << 20);
            let p: *mut IntCol = block.alloc_t(1);
            unsafe { IntCol::place(p, &block, &e) };
            let c = unsafe { &*p };
            let got: Vec<i64> = (0..v.len()).map(|i| c.get(i)).collect();
            assert_eq!(&got, v, "{}", c.tag_name());
            let mut each = Vec::new();
            c.for_each(|x| each.push(x));
            assert_eq!(&each, v);
        }
    }
}
