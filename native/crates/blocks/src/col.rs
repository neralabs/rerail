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

fn pack(vals: impl Iterator<Item = u64>, n: usize, bits: u32) -> Vec<u64> {
    let mut w = vec![0u64; (n * bits as usize).div_ceil(64) + 1];
    if bits == 0 {
        return w;
    }
    for (i, v) in vals.enumerate() {
        let bit = i * bits as usize;
        let (k, sh) = (bit >> 6, bit & 63);
        w[k] |= v << sh;
        if sh + bits as usize > 64 {
            w[k + 1] |= v >> (64 - sh);
        }
    }
    w
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

/// The smallest of the encodings for `v`.
pub fn encode(v: &[i64]) -> Encoded {
    let n = v.len();
    let none = Encoded { tag: CONST, bits: 0, rbits: 0, nruns: 0, base: 0, slope: 0, rmin: 0, n, data: vec![0], runs: Vec::new() };
    if n == 0 {
        return none;
    }
    let (lo, hi) = v.iter().fold((i64::MAX, i64::MIN), |(a, b), &x| (a.min(x), b.max(x)));
    if lo == hi {
        return Encoded { base: lo, ..none };
    }
    let bits = bits_for((hi as i128 - lo as i128) as u64);
    let pack_words = (n * bits as usize).div_ceil(64) + 1;
    let nruns = 1 + v.windows(2).filter(|w| w[0] != w[1]).count();
    let rbits = bits_for(n as u64);
    let runs_words = (nruns * bits as usize).div_ceil(64) + 1 + (nruns * rbits as usize).div_ceil(64) + 1;
    let slope = if n > 1 { (((v[n - 1] as i128 - v[0] as i128) << 16) / (n as i128 - 1)) as i64 } else { 0 };
    let (rlo, rhi) = (0..n).fold((i64::MAX, i64::MIN), |(a, b), i| {
        let r = v[i] - line(v[0], slope, i);
        (a.min(r), b.max(r))
    });
    let lbits = bits_for((rhi as i128 - rlo as i128) as u64);
    let line_words = (n * lbits as usize).div_ceil(64) + 1;
    let best = pack_words.min(runs_words).min(line_words);
    if best == runs_words {
        let starts: Vec<u64> = (0..n).filter(|&i| i == 0 || v[i] != v[i - 1]).map(|i| i as u64).collect();
        let vals = starts.iter().map(|&s| (v[s as usize] - lo) as u64);
        let data = pack(vals, nruns, bits);
        let runs = pack(starts.iter().copied(), nruns, rbits);
        return Encoded { tag: RUNS, bits, rbits, nruns: nruns as u32, base: lo, slope: 0, rmin: 0, n, data, runs };
    }
    if best == line_words && line_words < pack_words {
        let base = v[0];
        let data = pack((0..n).map(|i| (v[i] - line(base, slope, i) - rlo) as u64), n, lbits);
        return Encoded { tag: LINE, bits: lbits, rbits: 0, nruns: 0, base, slope, rmin: rlo, n, data, runs: Vec::new() };
    }
    let data = pack(v.iter().map(|&x| (x - lo) as u64), n, bits);
    Encoded { tag: PACK, bits, rbits: 0, nruns: 0, base: lo, slope: 0, rmin: 0, n, data, runs: Vec::new() }
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
            PACK => self.base + read(self.data.as_slice(), i, self.bits) as i64,
            RUNS => self.base + read(self.data.as_slice(), self.run_of(i), self.bits) as i64,
            _ => line(self.base, self.slope, i) + self.rmin + read(self.data.as_slice(), i, self.bits) as i64,
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
                    let v = self.base + read(d, k, self.bits) as i64;
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
                if self.base + (read(d, m, self.bits) as i64) < key { a = m + 1 } else { b = m }
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
