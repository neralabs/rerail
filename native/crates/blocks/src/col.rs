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
//! - `DICT` (decision 0622 step 4, MLDB's table): the column's distinct values, sorted, each less `base` in `bits` bits
//!   (`runs`, `nruns` of them), and each row's index among them in `rbits` bits (`data`). A column of few values
//!   spread wide (atoms, kinds, a few documents' ids): its codes take the bits its count needs, not its range. Taken
//!   for a relation's sealed chunks only (`for_reads`), where it is smaller than every other encoding.
//!
//! Reads are arithmetic on the words: the same on a GPU, where a column is read for its bandwidth and the extra
//! instructions to decode it cost nothing.

//! The encoder (the choice by exact size, the packers) is the floor's (`rerail_delta_core::flat::encode`), with no
//! heap; this module is its parallel driver (`Par`, the engine's cores) and the writers that place a column in a block.
use crate::block::Block;
// The format and its reads: the floor's one definition (`rerail_delta_core::col`).
pub use rerail_delta_core::col::{CONST, DICT, Dec, GROUPS, IntCol, LINE, PACK, RUNS, TAG_NAMES};
// The encoder: the floor's (*Flat Views and Lifecycles* step 4).
pub use rerail_delta_core::flat::encode::{Choice, Line, RUNS_FOR_READS, Stats, Vals};
use rerail_delta_core::flat::encode::{self as enc, Pieces, Serial};

/// Values a piece of a parallel pass holds. A multiple of 64, so a piece's values pack into whole words of its own.
pub(crate) const PIECE: usize = enc::PIECE;

/// The pieces of `0..n`, each `PIECE` values (the last the rest), run by `f` in parallel when there are several and
/// `par`; one after another when not (a caller whose own items are the parallel work: a twin's, decision 0686).
pub(crate) fn over_pieces<T: Send>(par: bool, n: usize, f: impl Fn(std::ops::Range<usize>) -> T + Sync) -> Vec<T> {
    let k = n.div_ceil(PIECE).max(1);
    if k == 1 {
        return vec![f(0..n)];
    }
    if !par {
        return (0..k).map(|p| f(p * PIECE..((p + 1) * PIECE).min(n))).collect();
    }
    use rayon::prelude::*;
    rerail_context::on_cores(|| (0..k).into_par_iter().map(|p| f(p * PIECE..((p + 1) * PIECE).min(n))).collect())
}

/// The host's driver of a pass: its pieces in parallel on the engine's cores, joined in order.
pub(crate) struct Par;

impl Pieces for Par {
    fn fold<T: Send>(&self, n: usize, f: impl Fn(std::ops::Range<usize>) -> T + Sync, join: impl Fn(T, T) -> T) -> T {
        over_pieces(true, n, f).into_iter().reduce(join).expect("a pass has a piece")
    }

    /// Every piece counted at once, their counts summed in order, then every piece written at once from its sum.
    fn scan(&self, n: usize, count: impl Fn(std::ops::Range<usize>) -> usize + Sync, each: impl Fn(std::ops::Range<usize>, usize) -> usize + Sync) {
        if enc::pieces(n) == 1 {
            each(0..n, 0);
            return;
        }
        let counts = over_pieces(true, n, count); // copy: not block data (the host's sums of its pieces' counts)
        let first: Vec<usize> = counts.iter().scan(0, |at, &k| Some(std::mem::replace(at, *at + k))).collect(); // copy: not block data (as `counts`)
        over_pieces(true, n, |r| each(r.clone(), first[r.start / PIECE]));
    }
}

/// Packs `vals` (each under `2^bits`) into `out`, which is zero and `words(vals.len(), bits)` long.
pub fn pack_into(out: &mut [u64], vals: &[u64], bits: u32) {
    enc::pack_into(out, vals, bits, &Par)
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

/// The smallest encoding of `v`: one pass for the range and the runs, one for the line's residuals (when a sample
/// says the line may win), each in pieces in parallel.
pub fn choose<V: Vals + ?Sized>(v: &V) -> Choice {
    enc::choose(v, usize::MAX, &Par, None)
}

/// `choose`, one piece after another: for a caller whose items are the parallel work.
pub fn choose_serial<V: Vals + ?Sized>(v: &V) -> Choice {
    enc::choose(v, usize::MAX, &Serial, None)
}

/// `v`'s encoding for a column read at random, by position (a relation's sealed chunk, decision 0622): every read a
/// few words. `RUNS` only with few runs, its read a search of their starts; `GROUPS` holds long runs about as small,
/// its read three words. `DICT` where it is smaller than all of them, its read two words. The floor's `IntFrom`
/// plans it, its distinct values counted in a work region of the host's; `f` is given the choice, the values and
/// `DICT`'s distinct values (empty for any other encoding).
pub fn for_reads<V: Vals, R>(v: V, f: impl FnOnce(&Choice, &V, &[i64]) -> R) -> R {
    use rerail_delta_core::flat::{Arena, Bucket, IntFrom, Region, arena::HEAD};
    let bytes = HEAD + enc::distinct_bytes(v.len());
    // The host's work room, never read before the arena's claims write it.
    let mut room: Vec<u64> = Vec::with_capacity(bytes.div_ceil(8));
    // SAFETY: the vector's room, `bytes` long and 8-byte aligned, held while the arena lives and nowhere else.
    let a = Arena::init(unsafe { Region::from_raw(room.as_mut_ptr().cast::<u8>(), bytes) }, Bucket::Phase).expect("a work region's arena");
    let plan = IntFrom::plan(v, Some(&a), 0).expect("`distinct_bytes` bounds what the count claims");
    f(plan.choice(), plan.vals(), plan.dict())
}

/// Packs `v` as `c` chose into `data` and `runs` (clear, `c.data_words` and `c.runs_words` long), in pieces in
/// parallel: the floor's writers (`encode::write`), on the engine's cores. `dict`: `DICT`'s distinct values, sorted
/// (`for_reads`'s); empty for any other encoding.
pub fn write<V: Vals + ?Sized>(c: &Choice, v: &V, data: &mut [u64], runs: &mut [u64], dict: &[i64]) {
    enc::write(c, v, data, runs, dict, &Par)
}

/// `write`, one piece after another: for a caller whose items are the parallel work.
pub fn write_serial<V: Vals + ?Sized>(c: &Choice, v: &V, data: &mut [u64], runs: &mut [u64], dict: &[i64]) {
    enc::write(c, v, data, runs, dict, &Serial)
}

/// The smallest of the encodings for `v`, packed.
pub fn encode(v: &[i64]) -> Encoded {
    let c = choose(v);
    let (mut data, mut runs) = (vec![0u64; c.data_words], vec![0u64; c.runs_words]);
    write(&c, v, &mut data, &mut runs, &[]);
    Encoded { tag: c.tag, bits: c.bits, rbits: c.rbits, nruns: c.nruns, base: c.base, slope: c.slope, rmin: c.rmin, n: c.n, data, runs }
}

/// A column's writes that take room from a block: its descriptor and its words, allocated there.
pub trait IntColW {
    /// Writes `e` into `block` at `this`.
    ///
    /// # Safety
    /// `this` is zeroed memory in `block`, which is open.
    unsafe fn place(this: *mut IntCol, block: &Block, e: &Encoded) {
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
                block.span(&raw const (*this).data, d, e.data.len());
            }
            if !e.runs.is_empty() {
                let r: *mut u64 = block.alloc_t(e.runs.len());
                std::ptr::copy_nonoverlapping(e.runs.as_ptr(), r, e.runs.len());
                block.span(&raw const (*this).runs, r, e.runs.len());
            }
        }
    }

    /// Writes `c`'s descriptor into `block` at `this` and allocates its words there, zero: the slices `write`
    /// packs the column into. Laid out first, columns can then be written in parallel.
    ///
    /// # Safety
    /// `this` is zeroed memory in `block`, which is open; the slices are written by one writer each.
    #[allow(clippy::mut_from_ref)]
    unsafe fn lay_out<'b>(this: *mut IntCol, block: &'b Block, c: &Choice) -> (&'b mut [u64], &'b mut [u64]) {
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
            block.span(&raw const (*this).data, d, c.data_words);
            let data = std::slice::from_raw_parts_mut(d, c.data_words);
            if c.runs_words == 0 {
                return (data, &mut []);
            }
            let r: *mut u64 = block.alloc_t(c.runs_words);
            block.span(&raw const (*this).runs, r, c.runs_words);
            (data, std::slice::from_raw_parts_mut(r, c.runs_words))
        }
    }

    /// Writes `c`'s descriptor at `this` with its words at `words` (`c.data_words`, then `c.runs_words`, zeroed),
    /// the slices `write` packs the column into: a descriptor and its words in one piece of a block, given back as
    /// one (a relation's sealed chunk, decision 0622).
    ///
    /// # Safety
    /// `this` and `words` are zeroed memory in `block`, `words` holding `c.data_words + c.runs_words` words.
    #[allow(clippy::mut_from_ref)]
    unsafe fn lay_out_at<'b>(this: *mut IntCol, block: &Block, words: *mut u64, c: &Choice) -> (&'b mut [u64], &'b mut [u64]) {
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
            block.span(&raw const (*this).data, words, c.data_words);
            let data = std::slice::from_raw_parts_mut(words, c.data_words);
            if c.runs_words == 0 {
                return (data, &mut []);
            }
            let r = words.add(c.data_words);
            block.span(&raw const (*this).runs, r, c.runs_words);
            (data, std::slice::from_raw_parts_mut(r, c.runs_words))
        }
    }
}

impl IntColW for IntCol {}

#[cfg(test)]
mod tests {
    use super::*;
    use rerail_delta_core::col::{GROUP, WIDTH_BITS, bits_for, words};
    use rerail_delta_core::flat::encode::range_of;
    use rerail_delta_core::col::read;

    /// `Stats`, one pass a piece over a column, takes `choose`'s encoding for every kind of column (constant, runs,
    /// a line, groups, packed, values near the ends of `i64`), across pieces (decision 0353 step 8).
    #[test]
    fn one_pass_statistics_choose_as_choose_does() {
        let n = PIECE * 2 + 777;
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        let mut rnd = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let cols: Vec<Vec<i64>> = vec![
            vec![7; n],
            (0..n as i64).map(|i| i / 1000).collect(),
            (0..n as i64).map(|i| 3 * i + 5).collect(),
            (0..n as i64).map(|i| 3 * i + (rnd() % 4) as i64).collect(),
            (0..n as i64).map(|i| (i / GROUP as i64) << 30 | (rnd() % 8) as i64).collect(),
            (0..n).map(|_| rnd() as i64 >> 20).collect(),
            (0..n).map(|_| rnd() as i64).collect(),
            vec![1, 2],
            vec![5],
        ];
        for v in &cols {
            let n = v.len();
            let parts = over_pieces(true, n, |r| {
                let mut st = Stats::piece(v[0], v[n - 1], n, (r.start > 0).then(|| v[r.start - 1]));
                for i in r {
                    st.push(i, v[i]);
                }
                st
            });
            let got = parts.into_iter().reduce(Stats::join).unwrap().choice(n);
            assert_eq!(format!("{got:?}"), format!("{:?}", choose(&v[..])), "a column of {n} from {}", v[0]);
        }
    }

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
            // A window of values at once, from any place (`Dec::fill`).
            let d = c.dec();
            for (lo, m) in [(0, v.len()), (1, v.len() - 1), (v.len() / 3, v.len() / 3), (v.len() - 1, 1)] {
                let mut out = vec![0i64; m];
                d.fill(lo, &mut out);
                assert_eq!(&out[..], &v[lo..lo + m], "{} from {lo}", c.tag_name());
            }
        }
    }

    /// Decision 0622 step 4: a column of few values spread wide is a `DICT` for reads at random (`for_reads`),
    /// smaller than every other encoding, and reads back by position, in order, and by search where sorted; across
    /// pieces packed in parallel, and at a code width of one bit. `choose` (the packed store's) takes none. A column
    /// of many values, or of few close together, is not one.
    #[test]
    fn a_column_of_few_values_spread_wide_is_a_dictionary() {
        let spread = [-(1i64 << 40), 7, 1 << 50, 3, i64::MAX - 5];
        let cases: Vec<Vec<i64>> = vec![
            (0..3 * PIECE as i64 + 11).map(|i| spread[(i * 2_654_435_761 % 5) as usize]).collect(),
            (0..1000i64).map(|i| if i % 3 == 0 { 1 << 45 } else { -9 }).collect(),
        ];
        for v in &cases {
            for_reads(&v[..], |c, _, dict| {
                assert_eq!(c.tag, DICT, "{c:?}");
                let others = choose(v);
                assert!(c.bytes() < others.bytes() && others.tag != DICT, "{c:?} against {others:?}");
                let block = crate::block::Block::anon(crate::block::kind::FACTS, 8 << 20);
                let p: *mut IntCol = block.alloc_t(1);
                let words: *mut u64 = block.alloc_t(c.data_words + c.runs_words);
                let (data, runs) = unsafe { IntCol::lay_out_at(p, &block, words, c) };
                write(c, v, data, runs, dict);
                let col = unsafe { &*p };
                assert!((0..v.len()).all(|i| col.get(i) == v[i] && col.dec().get(i) == v[i]), "a value read back");
                let mut each = Vec::new();
                col.for_each(|x| each.push(x));
                assert_eq!(&each, v);
                // A run of values decoded at once (`Dec::fill`), from the start and from within.
                for lo in [0, 1, v.len() / 2] {
                    let mut out = vec![0i64; (v.len() - lo).min(300)];
                    col.dec().fill(lo, &mut out);
                    assert_eq!(out, v[lo..lo + out.len()], "values {lo}.. filled");
                }
            });
        }
        // Sorted, it is a few runs, and `RUNS` is smaller: the same values written in the unsorted column's `DICT`
        // (the same values and count, so the same widths and the same dictionary) are searched by their values.
        let tag = |v: &[i64]| for_reads(v, |c, _, _| c.tag);
        let mut sorted = cases[0].clone();
        sorted.sort_unstable();
        assert_ne!(tag(&sorted), DICT);
        for_reads(&cases[0][..], |c, _, dict| {
            let block = crate::block::Block::anon(crate::block::kind::FACTS, 8 << 20);
            let p: *mut IntCol = block.alloc_t(1);
            let words: *mut u64 = block.alloc_t(c.data_words + c.runs_words);
            let (data, runs) = unsafe { IntCol::lay_out_at(p, &block, words, c) };
            write(c, &sorted, data, runs, dict);
            let col = unsafe { &*p };
            for key in spread.iter().copied().chain([0, i64::MIN]) {
                let want = (sorted.partition_point(|&x| x < key), sorted.partition_point(|&x| x <= key));
                assert_eq!(col.equal_range(0, sorted.len(), key), want, "{key}");
                assert_eq!(col.lower_bound(0, sorted.len(), key), want.0, "{key}");
            }
        });
        // Many values, or few within a small range: not a dictionary.
        assert_ne!(tag(&(0..5000i64).map(|i| i * 2_654_435_761 % 100_000).collect::<Vec<_>>()), DICT);
        assert_ne!(tag(&(0..5000i64).map(|i| i % 4).collect::<Vec<_>>()), DICT);
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
            write(&g, v, &mut data, &mut runs, &[]);
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
        let (lo, ghi) = v.chunks(GROUP).map(|g| range_of(g, 0..g.len())).fold((i64::MAX, i64::MIN), |(a, b), (x, _)| (a.min(x), b.max(x)));
        let gbits: usize = v.chunks(GROUP).map(|g| { let (x, y) = range_of(g, 0..g.len()); g.len() * bits_for((y as i128 - x as i128) as u64) as usize }).sum();
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
