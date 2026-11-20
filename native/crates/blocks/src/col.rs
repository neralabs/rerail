//! Frozen integer columns: written once, at freeze, in the fewest bits their values need (MLDB's
//! `RawMappedIntTable`: one small descriptor, the encoding chosen by exact size).
//!
//! - `CONST`: every value is `base`. No bytes.
//! - `PACK`: `v - base` in `bits` bits, packed into 64-bit words (frame of reference).
//! - `RUNS`: runs of equal values: each run's value (`v - base`, `bits` bits) and its first position (`rbits`).
//!   A sorted column with repeats, as a canonical order's leading column is.
//! - `LINE`: `v - (base + i * slope / 2^16)`, less `rmin`, in `bits` bits (MLDB's `Predictor`): a column that grows
//!   with its position, as ids in canonical order do.
//! - `GROUPS` (decision 0456 K2): groups of 128 values, each its own minimum (`bits` bits, less `base`) and width;
//!   a value is its group's minimum and its residual, packed at the group's width from the group's bit offset (each
//!   group's width and offset one `rbits`-bit entry). The random-access counterpart of a delta varint: a sorted id
//!   column's 128 values span a small range, and a group that crosses a document costs that group alone.
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
pub const GROUPS: u32 = 4;

/// A `GROUPS` group's values, as a shift.
const GROUP_LOG: u32 = 7;
const GROUP: usize = 1 << GROUP_LOG;
/// Bits of a group's width in its entry (widths 0..=64), below its offset.
const WIDTH_BITS: u32 = 7;

/// The encodings' names, by tag.
pub const TAG_NAMES: [&str; 5] = ["const", "pack", "runs", "line", "groups"];

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

/// Values a piece of a parallel pass holds. A multiple of 64, so a piece's values pack into whole words of its own.
const PIECE: usize = 1 << 16;

/// The pieces of `0..n`, each `PIECE` values (the last the rest), run by `f` in parallel when there are several.
fn over_pieces<T: Send>(n: usize, f: impl Fn(std::ops::Range<usize>) -> T + Sync) -> Vec<T> {
    let k = n.div_ceil(PIECE).max(1);
    if k == 1 {
        return vec![f(0..n)];
    }
    use rayon::prelude::*;
    (0..k).into_par_iter().map(|p| f(p * PIECE..((p + 1) * PIECE).min(n))).collect()
}

/// Packs `vals` (each under `2^bits`) into `out`, which is zero and `words(vals.len(), bits)` long.
pub fn pack_into(out: &mut [u64], vals: &[u64], bits: u32) {
    // SAFETY: `u64` and `i64` have one layout; the packer reads `v - 0` modulo 2^64, the same bits.
    let v = unsafe { std::slice::from_raw_parts(vals.as_ptr().cast::<i64>(), vals.len()) };
    pack_resid(out, v, Line { base: 0, slope: 0, rmin: 0 }, bits);
}

/// What a column's packed values are taken from: `v[i] - base - floor(i * slope / 2^16) - rmin`, modulo 2^64
/// (exact, as each is under `2^bits`). PACK is the line of slope 0.
#[derive(Clone, Copy)]
struct Line {
    base: i64,
    slope: i64,
    rmin: i64,
}

/// Packs `v`'s values less `line` into `out` (zero, `words(v.len(), bits)` long), the residuals computed as they
/// are packed (no pass to a temporary). A piece of `PIECE` values is `PIECE * bits / 64` whole words, so pieces
/// are packed in parallel and share no word.
fn pack_resid(out: &mut [u64], v: &[i64], line: Line, bits: u32) {
    if bits == 0 {
        return;
    }
    // A line of slope 0 is its base and minimum together: the packer with no line to subtract.
    let line = if line.slope == 0 { Line { base: line.base.wrapping_add(line.rmin), slope: 0, rmin: 0 } } else { line };
    let n = v.len();
    let full = n / PIECE;
    if full < 2 {
        return pack_piece(out, v, 0, line, bits);
    }
    use rayon::prelude::*;
    let per = PIECE * bits as usize / 64;
    let (head, rest) = out.split_at_mut(full * per);
    rayon::join(
        || head.par_chunks_mut(per).enumerate().for_each(|(k, o)| pack_piece(o, &v[k * PIECE..(k + 1) * PIECE], k * PIECE, line, bits)),
        || pack_piece(rest, &v[full * PIECE..], full * PIECE, line, bits),
    );
}

/// Packs `v` (positions `from..`, `from` a multiple of 64) into `out` from its first bit: a packer per width, and
/// per whether there is a line to subtract.
fn pack_piece(out: &mut [u64], v: &[i64], from: usize, line: Line, bits: u32) {
    macro_rules! widths {
        ($($b:literal)*) => {
            match (bits, line.slope != 0) {
                $(($b, false) => pack_fixed::<$b, false>(out, v, from, line),
                  ($b, true) => pack_fixed::<$b, true>(out, v, from, line),)*
                _ => unreachable!("{bits} bits"),
            }
        };
    }
    widths!(1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31 32
            33 34 35 36 37 38 39 40 41 42 43 44 45 46 47 48 49 50 51 52 53 54 55 56 57 58 59 60 61 62 63 64);
}

/// 64 values of `B` bits make `B` whole words, so a group of 64 is packed by a loop whose shifts and word
/// boundaries are constants once unrolled; the tail goes value by value.
#[inline(never)]
fn pack_fixed<const B: usize, const L: bool>(out: &mut [u64], v: &[i64], from: usize, line: Line) {
    // The line at the next value, `(from + j) * slope`, kept as a sum (the values go in order).
    let mut acc = from as i128 * line.slope as i128;
    let mut r = |x: i64| -> u64 {
        let x = x.wrapping_sub(line.base);
        if L {
            let t = (acc >> 16) as i64;
            acc += line.slope as i128;
            x.wrapping_sub(t).wrapping_sub(line.rmin) as u64
        } else {
            x as u64
        }
    };
    let groups = v.len() / 64;
    for g in 0..groups {
        let vg: &[i64; 64] = v[g * 64..g * 64 + 64].try_into().unwrap();
        let o: &mut [u64] = &mut out[g * B..g * B + B];
        for (j, &x) in vg.iter().enumerate() {
            let x = r(x);
            let bit = j * B;
            let (w, sh) = (bit / 64, bit % 64);
            o[w] |= x << sh;
            if sh + B > 64 {
                o[w + 1] |= x >> (64 - sh);
            }
        }
    }
    for (i, &x) in v.iter().enumerate().skip(groups * 64) {
        let x = r(x);
        let bit = i * B;
        let (w, sh) = (bit / 64, bit % 64);
        out[w] |= x << sh;
        if sh + B > 64 {
            out[w + 1] |= x >> (64 - sh);
        }
    }
}

/// Appends values of `bits` bits to zeroed words from a bit position, a word stored once it is full. Words are
/// ORed in atomically, so writers starting at other positions in the same words (a parallel pass's pieces, whose
/// first and last words they may share) never lose bits.
struct BitW<'a> {
    out: &'a [std::sync::atomic::AtomicU64],
    w: usize,
    fill: u32,
    acc: u64,
    bits: u32,
}

impl BitW<'_> {
    fn new(out: &[std::sync::atomic::AtomicU64], bit: usize, bits: u32) -> BitW<'_> {
        BitW { out, w: bit / 64, fill: (bit % 64) as u32, acc: 0, bits }
    }

    #[inline]
    fn push(&mut self, x: u64) {
        use std::sync::atomic::Ordering::Relaxed;
        self.acc |= x << self.fill;
        let end = self.fill + self.bits;
        if end >= 64 {
            self.out[self.w].fetch_or(self.acc, Relaxed);
            self.w += 1;
            self.acc = if self.fill == 0 { 0 } else { x >> (64 - self.fill) };
            self.fill = end - 64;
        } else {
            self.fill = end;
        }
    }

    fn flush(self) {
        if self.fill > 0 {
            self.out[self.w].fetch_or(self.acc, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

/// `words` as atomics, for writers that may share a word.
fn atomics(words: &mut [u64]) -> &[std::sync::atomic::AtomicU64] {
    // SAFETY: `AtomicU64` has `u64`'s size and alignment, and the borrow is exclusive.
    unsafe { &*(words as *mut [u64] as *const [std::sync::atomic::AtomicU64]) }
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

/// `v[i]`'s residual from the line through the first value at `slope`: `v[i] - (v[0] + i * slope / 2^16)`.
#[inline]
fn resid(v: &[i64], slope: i64, i: usize) -> i128 {
    v[i] as i128 - (v[0] as i128 + ((i as i128 * slope as i128) >> 16))
}

/// The line's residuals over `v`: the words they pack in, their bits and their minimum, or `usize::MAX` words
/// when the line cannot beat `bits` a value. `sample` is the residuals' range at every 64th row (taken in the
/// range's read): it is within the full one, so when it already needs `bits`, the full pass is skipped. The pass
/// runs in pieces in parallel, in `i64` when `slope * n` and the values (`lo..=hi`) cannot overflow it
/// (vectorized), else in `i128`.
fn line_fit(v: &[i64], slope: i64, bits: u32, lo: i64, hi: i64, sample: (i128, i128)) -> (usize, u32, i64) {
    let n = v.len();
    let no = (usize::MAX, 64, 0);
    let resid = |i: usize| resid(v, slope, i);
    let (slo, shi) = sample;
    if shi - slo > u64::MAX as i128 || bits_for((shi - slo) as u64) >= bits {
        return no;
    }
    let small = (slope as i128).abs() * n as i128 <= (1i128 << 62) && lo > -(1 << 61) && hi < 1 << 61;
    let parts = over_pieces(n, |r| {
        if small {
            let (v0, slope) = (v[0], slope);
            let (mut lo, mut hi, mut t) = (i64::MAX, i64::MIN, r.start as i64 * slope);
            for &x in &v[r] {
                let d = x - (v0 + (t >> 16));
                lo = lo.min(d);
                hi = hi.max(d);
                t += slope;
            }
            (lo as i128, hi as i128)
        } else {
            r.map(resid).fold((i128::MAX, i128::MIN), |(a, b), d| (a.min(d), b.max(d)))
        }
    });
    let (rlo, rhi) = parts.into_iter().fold((i128::MAX, i128::MIN), |(a, b), (c, d)| (a.min(c), b.max(d)));
    let range = rhi - rlo;
    if range > u64::MAX as i128 || rlo < i64::MIN as i128 {
        return no;
    }
    let lbits = bits_for(range as u64);
    (words(n, lbits), lbits, rlo as i64)
}

/// The least and greatest of `g` (not empty).
#[inline]
fn range_of(g: &[i64]) -> (i64, i64) {
    g.iter().fold((g[0], g[0]), |(a, b), &x| (a.min(x), b.max(x)))
}

/// The smallest encoding of `v`: one pass for the range and the runs, one for the line's residuals (when a sample
/// says the line may win), each in pieces in parallel.
pub fn choose(v: &[i64]) -> Choice {
    let n = v.len();
    let none = Choice { tag: CONST, bits: 0, rbits: 0, nruns: 0, base: 0, slope: 0, rmin: 0, n, data_words: 1, runs_words: 0 };
    if n == 0 {
        return none;
    }
    // The line through the first and last values (LINE's slope), known before any pass.
    let slope = if n > 1 { (((v[n - 1] as i128 - v[0] as i128) << 16) / (n as i128 - 1)) as i64 } else { 0 };
    // The range and the run count in one branch-free pass a piece (vectorized), and the line's residuals at
    // every 64th row (LINE's sample) in the same read.
    // The range a group at a time (a piece is whole groups): `GROUPS`' widths, summed, and its minima's range come
    // from the same read.
    let parts = over_pieces(n, |r| {
        let (mut a, mut b, mut gbits, mut ghi) = (i64::MAX, i64::MIN, 0usize, i64::MIN);
        for g in v[r.clone()].chunks(GROUP) {
            let (x, y) = range_of(g);
            (a, b, ghi) = (a.min(x), b.max(y), ghi.max(x));
            gbits += g.len() * bits_for((y as i128 - x as i128) as u64) as usize;
        }
        let from = r.start.max(1);
        let runs = (r.start == 0) as usize + v[from..r.end].iter().zip(&v[from - 1..]).map(|(a, b)| (a != b) as usize).sum::<usize>();
        let sample = (r.start.div_ceil(64) * 64..r.end).step_by(64).map(|i| resid(v, slope, i)).fold((i128::MAX, i128::MIN), |(a, b), d| (a.min(d), b.max(d)));
        (a, b, runs, sample, gbits, ghi)
    });
    let (lo, hi, nruns, sample, gbits, ghi) = parts.into_iter().fold(
        (i64::MAX, i64::MIN, 0, (i128::MAX, i128::MIN), 0, i64::MIN),
        |(a, b, r, s, g, h), (c, d, t, u, gb, gh)| (a.min(c), b.max(d), r + t, (s.0.min(u.0), s.1.max(u.1)), g + gb, h.max(gh)),
    );
    if lo == hi {
        return Choice { base: lo, ..none };
    }
    let bits = bits_for((hi as i128 - lo as i128) as u64);
    let pack_words = words(n, bits);
    let rbits = bits_for(n as u64);
    let runs = (words(nruns, bits), words(nruns, rbits));
    let (line_words, lbits, rlo) = line_fit(v, slope, bits, lo, hi, sample);
    // `GROUPS`: the residuals (two spare words: a group of width 0 at the end reads at the last bit), each group's
    // minimum less the column's (`lo`, the least of them), and each group's entry (width, offset).
    let ngroups = n.div_ceil(GROUP);
    let (gb, eb) = (bits_for((ghi as i128 - lo as i128) as u64), WIDTH_BITS + bits_for(gbits as u64));
    let groups = (gbits / 64 + 2, words(ngroups, gb) + words(ngroups, eb));
    let best = pack_words.min(runs.0 + runs.1).min(line_words).min(groups.0 + groups.1);
    if best == groups.0 + groups.1 && best < pack_words.min(runs.0 + runs.1).min(line_words) {
        return Choice { tag: GROUPS, bits: gb, rbits: eb, nruns: ngroups as u32, base: lo, slope: 0, rmin: 0, n, data_words: groups.0, runs_words: groups.1 };
    }
    if best == runs.0 + runs.1 {
        return Choice { tag: RUNS, bits, rbits, nruns: nruns as u32, base: lo, slope: 0, rmin: 0, n, data_words: runs.0, runs_words: runs.1 };
    }
    if best == line_words && line_words < pack_words {
        return Choice { tag: LINE, bits: lbits, rbits: 0, nruns: 0, base: v[0], slope, rmin: rlo, n, data_words: line_words, runs_words: 0 };
    }
    Choice { tag: PACK, bits, rbits: 0, nruns: 0, base: lo, slope: 0, rmin: 0, n, data_words: pack_words, runs_words: 0 }
}

/// Packs `v` as `c` chose into `data` and `runs` (zero, `c.data_words` and `c.runs_words` long), in pieces in
/// parallel. RUNS counts each piece's runs, so each piece knows its first run, then writes its runs from there.
pub fn write(c: &Choice, v: &[i64], data: &mut [u64], runs: &mut [u64]) {
    match c.tag {
        CONST => {}
        PACK => pack_resid(data, v, Line { base: c.base, slope: 0, rmin: 0 }, c.bits),
        LINE => pack_resid(data, v, Line { base: c.base, slope: c.slope, rmin: c.rmin }, c.bits),
        GROUPS => write_groups(c, v, data, runs),
        _ => {
            let counts = over_pieces(v.len(), |r| {
                let from = r.start.max(1);
                (r.start == 0) as usize + v[from..r.end].iter().zip(&v[from - 1..]).map(|(a, b)| (a != b) as usize).sum::<usize>()
            });
            let (d, s) = (atomics(data), atomics(runs));
            let mut first = Vec::with_capacity(counts.len());
            let mut at = 0;
            for k in counts {
                first.push(at);
                at += k;
            }
            over_pieces(v.len(), |r| {
                let k = first[r.start / PIECE];
                let (mut wv, mut ws) = (BitW::new(d, k * c.bits as usize, c.bits), BitW::new(s, k * c.rbits as usize, c.rbits));
                let mut push = |i: usize, x: i64| {
                    wv.push(x.wrapping_sub(c.base) as u64);
                    ws.push(i as u64);
                };
                if r.start == 0 {
                    push(0, v[0]);
                }
                let from = r.start.max(1);
                for (j, (&x, &prev)) in v[from..r.end].iter().zip(&v[from - 1..]).enumerate() {
                    if x != prev {
                        push(from + j, x);
                    }
                }
                wv.flush();
                ws.flush();
            });
        }
    }
}

/// `GROUPS`' words: each group's minimum and width (a pass in pieces), their offsets (a prefix sum), then in pieces
/// in parallel each group's residuals into `data` and its minimum and entry into `runs`.
fn write_groups(c: &Choice, v: &[i64], data: &mut [u64], runs: &mut [u64]) {
    let n = v.len();
    let gs: Vec<(i64, u32)> = over_pieces(n, |r| {
        v[r].chunks(GROUP)
            .map(|g| {
                let (x, y) = range_of(g);
                (x, bits_for((y as i128 - x as i128) as u64))
            })
            .collect::<Vec<_>>()
    })
    .concat();
    let mut offs = Vec::with_capacity(gs.len());
    let mut at = 0usize;
    for (k, &(_, w)) in gs.iter().enumerate() {
        offs.push(at);
        at += w as usize * GROUP.min(n - k * GROUP);
    }
    let (d, s) = (atomics(data), atomics(runs));
    let entries = words(gs.len(), c.bits) * 64;
    over_pieces(n, |r| {
        let (k0, k1) = (r.start >> GROUP_LOG, r.end.div_ceil(GROUP));
        let mut mins = BitW::new(s, k0 * c.bits as usize, c.bits);
        let mut ents = BitW::new(s, entries + k0 * c.rbits as usize, c.rbits);
        for k in k0..k1 {
            let (gmin, w) = gs[k];
            if c.bits > 0 {
                mins.push(gmin.wrapping_sub(c.base) as u64);
            }
            ents.push((offs[k] as u64) << WIDTH_BITS | w as u64);
            if w > 0 {
                let mut out = BitW::new(d, offs[k], w);
                for &x in &v[k * GROUP..((k + 1) * GROUP).min(n)] {
                    out.push(x.wrapping_sub(gmin) as u64);
                }
                out.flush();
            }
        }
        mins.flush();
        ents.flush();
    });
}

/// The smallest of the encodings for `v`, packed.
pub fn encode(v: &[i64]) -> Encoded {
    let c = choose(v);
    let (mut data, mut runs) = (vec![0u64; c.data_words], vec![0u64; c.runs_words]);
    write(&c, v, &mut data, &mut runs);
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

    /// Writes `c`'s descriptor into `block` at `this` and allocates its words there, zero: the slices `write`
    /// packs the column into. Laid out first, columns can then be written in parallel.
    ///
    /// # Safety
    /// `this` is zeroed memory in `block`, which is open; the slices are written by one writer each.
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn lay_out<'b>(this: *mut IntCol, block: &'b Block, c: &Choice) -> (&'b mut [u64], &'b mut [u64]) {
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
                return (&mut [], &mut []);
            }
            let d: *mut u64 = block.alloc_t(c.data_words);
            Span::set(&raw mut (*this).data, d, c.data_words);
            let data = std::slice::from_raw_parts_mut(d, c.data_words);
            if c.runs_words == 0 {
                return (data, &mut []);
            }
            let r: *mut u64 = block.alloc_t(c.runs_words);
            Span::set(&raw mut (*this).runs, r, c.runs_words);
            (data, std::slice::from_raw_parts_mut(r, c.runs_words))
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.n as usize
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// The column resolved for reads: its fields and words loaded once.
    #[inline]
    pub fn dec(&self) -> Dec<'_> {
        let mask = |b: u32| if b == 0 { 0 } else if b == 64 { u64::MAX } else { (1u64 << b) - 1 };
        Dec {
            tag: self.tag,
            bits: self.bits,
            mask: mask(self.bits),
            rbits: self.rbits,
            rmask: mask(self.rbits),
            n: self.n as usize,
            nruns: self.nruns as usize,
            entries: if self.tag == GROUPS { words(self.nruns as usize, self.bits) * 64 } else { 0 },
            base: self.base,
            slope: self.slope,
            rmin: self.rmin,
            data: self.data.as_slice(),
            runs: self.runs.as_slice(),
        }
    }

    #[inline]
    fn run_start(&self, k: usize) -> usize {
        read(self.runs.as_slice(), k, self.rbits) as usize
    }

    #[inline]
    pub fn get(&self, i: usize) -> i64 {
        match self.tag {
            CONST => self.base,
            // PACK takes at least one bit.
            PACK => self.base.wrapping_add(rd(self.data.as_slice(), i, self.bits, u64::MAX >> (64 - self.bits)) as i64),
            _ => self.dec().get(i),
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
    pub fn lower_bound(&self, lo: usize, hi: usize, key: i64) -> usize {
        self.dec().lower_bound(lo, hi, key)
    }

    /// `lo..hi`'s positions whose value is `key`, the column sorted over that range.
    pub fn equal_range(&self, lo: usize, hi: usize, key: i64) -> (usize, usize) {
        self.dec().equal_range(lo, hi, key)
    }

    pub fn bytes(&self) -> usize {
        (self.data.len() + self.runs.len()) * 8
    }

    pub fn tag_name(&self) -> &'static str {
        TAG_NAMES[self.tag as usize]
    }
}

/// Value `i` of `bits` bits (`mask` its mask, `bits > 0`) from packed words: two loads and two shifts, no branch.
#[inline(always)]
fn rd(words: &[u64], i: usize, bits: u32, mask: u64) -> u64 {
    let bit = i * bits as usize;
    let (w, sh) = (bit >> 6, (bit & 63) as u32);
    debug_assert!(w + 1 < words.len());
    // SAFETY: a column's words end with a spare one (`words`), so `w + 1` is in bounds for every value it holds.
    let (a, b) = unsafe { (*words.get_unchecked(w), *words.get_unchecked(w + 1)) };
    // `b << 1 << (63 - sh)` is `b << (64 - sh)` without a shift by 64 when `sh` is 0.
    ((a >> sh) | ((b << 1) << (63 - sh))) & mask
}

/// The value of `mask`'s bits at bit `bit` of packed words: `rd` from a bit position.
#[inline(always)]
fn rd_bit(words: &[u64], bit: usize, mask: u64) -> u64 {
    let (w, sh) = (bit >> 6, (bit & 63) as u32);
    debug_assert!(w + 1 < words.len());
    // SAFETY: `GROUPS`' words end with two spare ones, so a read at its last bit (a group of width 0 there) is in
    // bounds; its entries' words end with the spare one `words` gives.
    let (a, b) = unsafe { (*words.get_unchecked(w), *words.get_unchecked(w + 1)) };
    ((a >> sh) | ((b << 1) << (63 - sh))) & mask
}

/// The mask of `w` bits, `w` up to 64: no branch.
#[inline(always)]
fn mask_of(w: u32) -> u64 {
    ((1u128 << w) - 1) as u64
}

/// The first position in `lo..hi` whose value (`f`, non-decreasing) is not less than `key`. One guess by
/// interpolation between the ends, a gallop from it to bracket the answer, then a binary search of the bracket.
/// A sorted column of a packed block is near linear (keys, run values, leading columns, starts), so the guess
/// lands within a few positions and a search reads about six values where a binary one reads `log2 n`. A
/// skewed column costs at most about twice the binary search.
#[inline]
fn search(mut lo: usize, mut hi: usize, key: i64, f: impl Fn(usize) -> i64) -> usize {
    if hi - lo > 8 {
        let (a, b) = (f(lo), f(hi - 1));
        if key <= a {
            return lo;
        }
        if key > b {
            return hi;
        }
        // f(lo) < key <= f(hi - 1): the answer is in lo + 1..=hi - 1.
        let g = lo + ((key as i128 - a as i128) * (hi - 1 - lo) as i128 / (b as i128 - a as i128)) as usize;
        (lo, hi) = (lo + 1, hi - 1);
        let g = g.clamp(lo, hi);
        let mut step = 1;
        if f(g) < key {
            lo = g + 1;
            let mut p = g;
            while p + step < hi {
                let q = p + step;
                if f(q) < key {
                    (lo, p, step) = (q + 1, q, step << 1);
                } else {
                    hi = q;
                    break;
                }
            }
        } else {
            hi = g;
            let mut p = g;
            while p - lo >= step {
                let q = p - step;
                if f(q) >= key {
                    (hi, p, step) = (q, q, step << 1);
                } else {
                    lo = q + 1;
                    break;
                }
            }
        }
    }
    while lo < hi {
        let m = (lo + hi) / 2;
        if f(m) < key { lo = m + 1 } else { hi = m }
    }
    lo
}

/// The end of `key`'s positions from `a` (whose value is `key`) in `a..hi`: a gallop, then a binary search.
#[inline]
fn upper_from(a: usize, hi: usize, key: i64, f: impl Fn(usize) -> i64) -> usize {
    let (mut lo, mut end, mut p, mut step) = (a + 1, hi, a, 1);
    while p + step < hi {
        let q = p + step;
        if f(q) == key {
            (lo, p, step) = (q + 1, q, step << 1);
        } else {
            end = q;
            break;
        }
    }
    while lo < end {
        let m = (lo + end) / 2;
        if f(m) == key { lo = m + 1 } else { end = m }
    }
    lo
}

/// An `IntCol` resolved for reads: its descriptor's fields and its words' slices loaded once, so a probe's reads
/// are arithmetic on registers (a descriptor's spans are atomics, which a compiler does not merge).
#[derive(Clone, Copy)]
pub struct Dec<'a> {
    tag: u32,
    bits: u32,
    mask: u64,
    rbits: u32,
    rmask: u64,
    n: usize,
    nruns: usize,
    /// `GROUPS`: the bit where the groups' entries start in `runs`, past their minima.
    entries: usize,
    base: i64,
    slope: i64,
    rmin: i64,
    data: &'a [u64],
    runs: &'a [u64],
}

impl Dec<'_> {
    #[inline]
    pub fn len(&self) -> usize {
        self.n
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// Packed value `k` (a value of `PACK`, a run's value of `RUNS`).
    #[inline(always)]
    fn val(&self, k: usize) -> i64 {
        self.base.wrapping_add(rd(self.data, k, self.bits, self.mask) as i64)
    }

    /// `LINE`'s value `i`: the line, its residuals' minimum and the residual, exact modulo 2^64.
    #[inline(always)]
    fn on_line(&self, i: usize) -> i64 {
        let r = if self.bits == 0 { 0 } else { rd(self.data, i, self.bits, self.mask) };
        let t = ((i as i128 * self.slope as i128) >> 16) as i64;
        self.base.wrapping_add(t).wrapping_add(self.rmin).wrapping_add(r as i64)
    }

    /// `GROUPS`' value `i`: its group's minimum, then its residual at the group's width from the group's offset.
    #[inline(always)]
    fn grp(&self, i: usize) -> i64 {
        let k = i >> GROUP_LOG;
        let min = rd_bit(self.runs, k * self.bits as usize, self.mask);
        let e = rd_bit(self.runs, self.entries + k * self.rbits as usize, self.rmask);
        let w = (e & ((1 << WIDTH_BITS) - 1)) as u32;
        let r = rd_bit(self.data, (e >> WIDTH_BITS) as usize + (i & (GROUP - 1)) * w as usize, mask_of(w));
        self.base.wrapping_add(min as i64).wrapping_add(r as i64)
    }

    /// Run `k`'s first position (`n` past the last run).
    #[inline(always)]
    fn start(&self, k: usize) -> usize {
        if k >= self.nruns { self.n } else { rd(self.runs, k, self.rbits, self.rmask) as usize }
    }

    /// The run holding position `i`.
    #[inline]
    fn run_of(&self, i: usize) -> usize {
        let (mut lo, mut hi) = (0usize, self.nruns);
        while hi - lo > 1 {
            let mid = (lo + hi) / 2;
            if self.start(mid) <= i { lo = mid } else { hi = mid }
        }
        lo
    }

    /// The runs `lo..hi` covers.
    #[inline]
    fn runs_over(&self, lo: usize, hi: usize) -> (usize, usize) {
        let a = if lo == 0 { 0 } else { self.run_of(lo) };
        let b = if hi >= self.n { self.nruns } else { self.run_of(hi - 1) + 1 };
        (a, b)
    }

    #[inline]
    pub fn get(&self, i: usize) -> i64 {
        match self.tag {
            CONST => self.base,
            PACK => self.val(i),
            RUNS => self.val(self.run_of(i)),
            GROUPS => self.grp(i),
            _ => self.on_line(i),
        }
    }

    /// The first position in `lo..hi` whose value is not less than `key`, the column sorted over that range.
    #[inline]
    pub fn lower_bound(&self, lo: usize, hi: usize, key: i64) -> usize {
        if lo >= hi {
            return lo;
        }
        match self.tag {
            CONST => if self.base < key { hi } else { lo },
            PACK => search(lo, hi, key, |i| self.val(i)),
            RUNS => {
                // A run at a time: the search reads the runs' values, not the rows.
                let (a, b) = self.runs_over(lo, hi);
                self.start(search(a, b, key, |k| self.val(k))).clamp(lo, hi)
            }
            GROUPS => search(lo, hi, key, |i| self.grp(i)),
            _ => search(lo, hi, key, |i| self.on_line(i)),
        }
    }

    /// `lo..hi`'s positions whose value is `key`, the column sorted over that range: one search, and the end
    /// found from the start (a run's end, or a gallop over a key's few rows).
    #[inline]
    pub fn equal_range(&self, lo: usize, hi: usize, key: i64) -> (usize, usize) {
        if lo >= hi {
            return (lo, lo);
        }
        match self.tag {
            CONST => {
                let p = self.lower_bound(lo, hi, key);
                if self.base == key { (lo, hi) } else { (p, p) }
            }
            PACK => {
                let a = search(lo, hi, key, |i| self.val(i));
                if a == hi || self.val(a) != key { (a, a) } else { (a, upper_from(a, hi, key, |i| self.val(i))) }
            }
            RUNS => {
                let (ra, rb) = self.runs_over(lo, hi);
                let r = search(ra, rb, key, |k| self.val(k));
                let at = self.start(r).clamp(lo, hi);
                if r == rb || self.val(r) != key { (at, at) } else { (at, self.start(r + 1).min(hi)) }
            }
            GROUPS => {
                let a = search(lo, hi, key, |i| self.grp(i));
                if a == hi || self.grp(a) != key { (a, a) } else { (a, upper_from(a, hi, key, |i| self.grp(i))) }
            }
            _ => {
                let a = search(lo, hi, key, |i| self.on_line(i));
                if a == hi || self.on_line(a) != key { (a, a) } else { (a, upper_from(a, hi, key, |i| self.on_line(i))) }
            }
        }
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

    #[test]
    fn parallel_pieces_pack_as_one_thread_does() {
        let n = 5 * PIECE + 77;
        let mut x = 0x2545_F491_4F6C_DD1Du64;
        let mut rnd = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let cases: Vec<Vec<i64>> = vec![
            (0..n).map(|_| (rnd() % 100_000) as i64 - 5).collect(),
            (0..n as i64).map(|i| i / 37).collect(),
            (0..n as i64).map(|i| -7_000_000 + i * 3 + (rnd() % 5) as i64).collect(),
            (0..n).map(|_| 1_000 + (rnd() % 2) as i64).collect(),
            (0..n).map(|_| rnd() as i64).collect(),
        ];
        let one = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
        let many = rayon::ThreadPoolBuilder::new().num_threads(4).build().unwrap();
        for v in &cases {
            let (a, b) = (one.install(|| encode(v)), many.install(|| encode(v)));
            assert_eq!((a.tag, a.bits, a.nruns), (b.tag, b.bits, b.nruns));
            assert!(a.data == b.data && a.runs == b.runs, "{} words differ", a.tag);
            let block = crate::block::Block::anon(crate::block::kind::FACTS, 8 << 20);
            let p: *mut IntCol = block.alloc_t(1);
            unsafe { IntCol::place(p, &block, &b) };
            let c = unsafe { &*p };
            assert!((0..n).all(|i| c.get(i) == v[i]), "{} reads back", c.tag_name());
        }
        let tags: Vec<u32> = cases.iter().map(|v| encode(v).tag).collect();
        assert_eq!(&tags[..3], &[PACK, RUNS, LINE]);
    }

    /// K-R1 (decision 0456): `GROUPS` reads back every value, is chosen for node ids clustered by document (sorted,
    /// and in a run's order), and searches as a scan does.
    #[test]
    fn groups_reads_every_value() {
        let mut x = 0x2545_F491_4F6C_DD1Du64;
        let mut rnd = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        // Node ids of 300 documents, each 40..2,000 nodes with gaps: `document << 32 | slot << 18 | index`.
        let mut sorted = Vec::new();
        for d in 0..300i64 {
            let mut low = 0i64;
            for _ in 0..40 + rnd() % 1960 {
                low += 1 + (rnd() % 3) as i64;
                sorted.push((d + 17) << 32 | low);
            }
        }
        // The same ids in a run's order: grouped by document, shuffled within each.
        let mut clustered = sorted.clone();
        for c in clustered.chunk_by_mut(|a, b| a >> 32 == b >> 32) {
            for i in (1..c.len()).rev() {
                c.swap(i, (rnd() as usize) % (i + 1));
            }
        }
        let n = sorted.len();
        let random: Vec<i64> = (0..3 * PIECE + 5).map(|_| rnd() as i64).collect();
        let edge: Vec<i64> = (0..1000).map(|i| if i % 128 < 64 { i64::MIN + i } else { i64::MAX - i }).collect();
        // Sorted: a group's 128 ids span about 256 (9 bits, and its entry); shuffled within a document, about the
        // document's size (up to 12 bits). An 8-byte cell is 64.
        for (v, most) in [(&sorted, 12), (&clustered, 15)] {
            let e = encode(v);
            assert_eq!(e.tag, GROUPS, "ids clustered by document");
            assert!(e.bytes() * 8 < most * v.len(), "{} bits a value", e.bytes() * 8 / v.len());
        }
        for v in [&sorted, &clustered, &random, &edge] {
            let c = choose(v);
            let mut data = vec![0u64; groups_words(v).0];
            let mut runs = vec![0u64; groups_words(v).1];
            let g = Choice { tag: GROUPS, data_words: data.len(), runs_words: runs.len(), ..groups_choice(v) };
            write(&g, v, &mut data, &mut runs);
            let block = crate::block::Block::anon(crate::block::kind::FACTS, 64 << 20);
            let p: *mut IntCol = block.alloc_t(1);
            let e = Encoded { tag: g.tag, bits: g.bits, rbits: g.rbits, nruns: g.nruns, base: g.base, slope: 0, rmin: 0, n: g.n, data, runs };
            unsafe { IntCol::place(p, &block, &e) };
            let col = unsafe { &*p };
            assert!((0..v.len()).all(|i| col.get(i) == v[i]), "groups reads back ({} chose {})", v.len(), TAG_NAMES[c.tag as usize]);
        }
        let block = crate::block::Block::anon(crate::block::kind::FACTS, 16 << 20);
        let p: *mut IntCol = block.alloc_t(1);
        unsafe { IntCol::place(p, &block, &encode(&sorted)) };
        let d = unsafe { &*p }.dec();
        for _ in 0..2000 {
            let (a, b) = ((rnd() as usize) % (n + 1), (rnd() as usize) % (n + 1));
            let (lo, hi) = (a.min(b), a.max(b));
            let key = if rnd() % 2 == 0 && lo < hi { sorted[lo + (rnd() as usize) % (hi - lo)] } else { sorted[(rnd() as usize) % n] + 1 };
            let want = lo + sorted[lo..hi].partition_point(|&y| y < key);
            let end = lo + sorted[lo..hi].partition_point(|&y| y <= key);
            assert_eq!(d.equal_range(lo, hi, key), (want, end), "{lo}..{hi} {key}");
        }
    }

    /// `GROUPS`' choice for `v`, whichever encoding `choose` takes.
    fn groups_choice(v: &[i64]) -> Choice {
        let (lo, ghi) = v.chunks(GROUP).map(range_of).fold((i64::MAX, i64::MIN), |(a, b), (x, _)| (a.min(x), b.max(x)));
        let gbits: usize = v.chunks(GROUP).map(|g| { let (x, y) = range_of(g); g.len() * bits_for((y as i128 - x as i128) as u64) as usize }).sum();
        let ng = v.len().div_ceil(GROUP);
        let (gb, eb) = (bits_for((ghi as i128 - lo as i128) as u64), WIDTH_BITS + bits_for(gbits as u64));
        Choice { tag: GROUPS, bits: gb, rbits: eb, nruns: ng as u32, base: lo, slope: 0, rmin: 0, n: v.len(), data_words: gbits / 64 + 2, runs_words: words(ng, gb) + words(ng, eb) }
    }

    fn groups_words(v: &[i64]) -> (usize, usize) {
        let c = groups_choice(v);
        (c.data_words, c.runs_words)
    }

    #[test]
    fn searches_agree_with_a_scan() {
        let mut x = 0x9E37_79B9_7F4A_7C15u64;
        let mut rnd = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        // Sorted columns of each encoding, near linear and skewed, with ranges inside them.
        let mut skew: Vec<i64> = (0..3000).map(|i: i64| i * i * i / 1000 + (rnd() % 3) as i64).collect();
        skew.sort();
        let cases: Vec<Vec<i64>> = vec![
            vec![7; 50],
            (0..3000).collect(),
            (0..3000).map(|i| i / 7).collect(),
            (0..3000).map(|i| 5 * i + (rnd() % 4) as i64).collect(),
            skew,
            (0..3000).map(|i| if i < 2990 { 0 } else { 1 << 40 }).collect(),
        ];
        for v in &cases {
            let e = encode(v);
            let block = crate::block::Block::anon(crate::block::kind::FACTS, 1 << 20);
            let p: *mut IntCol = block.alloc_t(1);
            unsafe { IntCol::place(p, &block, &e) };
            let d = unsafe { &*p }.dec();
            let n = v.len();
            for _ in 0..400 {
                let (a, b) = ((rnd() as usize) % (n + 1), (rnd() as usize) % (n + 1));
                let (lo, hi) = (a.min(b), a.max(b));
                let key = match rnd() % 3 {
                    0 if lo < hi => v[lo + (rnd() as usize) % (hi - lo)],
                    1 => v[(rnd() as usize) % n] + 1,
                    _ => v[0] - 2 + (rnd() % ((v[n - 1] - v[0]) as u64 + 4)) as i64,
                };
                let want = lo + v[lo..hi].partition_point(|&y| y < key);
                let end = lo + v[lo..hi].partition_point(|&y| y <= key);
                assert_eq!(d.lower_bound(lo, hi, key), want, "{} {lo}..{hi} {key}", e.tag);
                assert_eq!(d.equal_range(lo, hi, key), (want, end), "{} {lo}..{hi} {key}", e.tag);
            }
        }
    }
}
