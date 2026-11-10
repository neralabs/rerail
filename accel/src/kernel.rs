//! The kernels: each physical form of decision 0098 §1 as a function of plain columns, with one meaning.
//!
//! Inputs are shared (`Arc`), so a batch can hold them without copying and a lane can read them in place (the GPU
//! wraps them as no-copy buffers when they are page-aligned, and copies them otherwise).
//!
//! Partitions: several kernels work per partition (a document, a key run). A partitioned column comes with offsets,
//! `nparts + 1` of them, ascending, the first 0 and the last the column's length. Partition `p` is
//! `col[off[p]..off[p + 1]]`.

use crate::lanes::Lane;
use std::sync::Arc;

/// Rows of bits, packed: row `r` is `data[r * words..(r + 1) * words]`, bit `c` of a row in word `c / 64`, bit
/// `c % 64`. Bits past the row's width are zero.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bits {
    pub rows: usize,
    pub words: usize,
    pub data: Arc<[u64]>,
    /// Set when the rows are a **fixed relation**: the same across many runs (a pack's rule vocabularies, library
    /// facts held once). It is the digest of the rows' content, and keys whatever a lane builds from them once (the
    /// Neural Engine compiles them into a model as a constant). `None` for rows made per run.
    pub fixed: Option<[u8; 32]>,
}

impl Bits {
    pub fn new(rows: usize, words: usize, data: impl Into<Arc<[u64]>>) -> Self {
        let data = data.into();
        assert_eq!(data.len(), rows * words, "bits: {rows} rows of {words} words");
        Bits { rows, words, data, fixed: None }
    }

    pub fn zeros(rows: usize, words: usize) -> Self {
        Bits::new(rows, words, vec![0u64; rows * words])
    }

    /// The same rows, marked as a fixed relation by the digest of their content.
    pub fn fixed(mut self) -> Self {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update((self.rows as u64).to_le_bytes());
        h.update((self.words as u64).to_le_bytes());
        for w in self.data.iter() {
            h.update(w.to_le_bytes());
        }
        self.fixed = Some(h.finalize().into());
        self
    }

    pub fn row(&self, r: usize) -> &[u64] {
        &self.data[r * self.words..(r + 1) * self.words]
    }

    pub fn get(&self, r: usize, c: usize) -> bool {
        self.row(r)[c / 64] >> (c % 64) & 1 == 1
    }
}

/// A kernel its caller makes (decision 0100 §13): a program of the caller's own, with a form per lane it runs on (a
/// device's encoding, the cores' interpreter). It is a launch like any other: priced by its own per-unit seconds,
/// placed, claimed by whichever lane reaches it, and scored. A device's form submits on the lane's worker, through the
/// one device and queue (`gpu::shared`).
pub trait Program: Send + Sync {
    /// Its name, for the scoreboard and the profile.
    fn name(&self) -> &'static str;
    /// Its size, in the unit its prices are per (bytes, for a program over text).
    fn units(&self) -> f64;
    /// Seconds a unit on `lane`; `None` when it has no form there.
    fn per_unit(&self, lane: Lane) -> Option<f64>;
    /// Host CPU seconds a unit on `lane` (on the cores, all of it; on a device, its encoding and reads).
    fn host_per_unit(&self, lane: Lane) -> Option<f64> {
        let per = self.per_unit(lane)?;
        Some(if matches!(lane, Lane::Gpu) { per.min(1e-10) } else { per })
    }
    /// Runs it on `lane`. A device's form that cannot take it (its capacity, a shape it lacks) runs the cores' form
    /// instead, so the launch does not fail for it.
    fn run(&self, lane: Lane) -> Result<ProgramOutput, String>;
}

/// A program's result, the caller's own type ([`Program`]).
pub type ProgramOutput = Arc<dyn std::any::Any + Send + Sync>;

/// A program's kernel ([`Kernel::Program`]).
#[derive(Clone)]
pub struct ProgramKernel(pub Arc<dyn Program>);

impl std::fmt::Debug for ProgramKernel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Program({}, {} units)", self.0.name(), self.0.units())
    }
}

/// A launch's work: one physical form over its columns.
#[derive(Clone, Debug)]
pub enum Kernel {
    /// **Bit product.** For each row `i` of `a` and row `j` of `b`, `popcount(a[i] & b[j])`: how many members two
    /// sets share. `a` and `b` have the same `words`.
    ///
    /// With `min`, only the pairs whose count reaches `min[j]` are kept, as [`Output::Pairs`] in `(i, j)` order:
    /// a semi-join (`min` all 1), or the prefilter's "every one of the rule's names occurs" (`min[j]` the size of
    /// rule `j`). Without it, every count, as [`Output::Counts`] (`a.rows` × `b.rows`).
    BitProduct { a: Bits, b: Bits, min: Option<Arc<[u32]>> },

    /// **Count table.** An equality join within each partition, on 16-bit codes (decision 0095 §6: a document's
    /// local name ids, a kind, a node's low 16 bits).
    ///
    /// For each left row, how many right rows of the same partition have its code: [`Output::PerRow`]
    /// (`left.len()` counts). With `emit`, the pairs themselves instead, as [`Output::Pairs`] of (left row, right
    /// row), both indexes into the whole columns, in left order and then right order. `loff` and `roff` have the same
    /// number of partitions.
    CountTable { loff: Arc<[u32]>, left: Arc<[u16]>, roff: Arc<[u32]>, right: Arc<[u16]>, emit: bool },

    /// **Histogram.** A group-by count per partition on a code below `width`: [`Output::Counts`] of `nparts` ×
    /// `width`.
    Histogram { off: Arc<[u32]>, codes: Arc<[u16]>, width: u32 },

    /// **Closure.** The transitive closure of a graph of `n` nodes, its edges as CSR (`off`: `n + 1` offsets into
    /// `dst`). Node `j` is in row `i` of the [`Output::Bits`] when a path of one or more edges leads from `i` to `j`
    /// (so `i` is in its own row only on a cycle).
    Closure { n: u32, off: Arc<[u32]>, dst: Arc<[u32]> },

    /// **Join.** The equi-join of two relations on their key columns, key `i` each side's column `i` (decision 0100
    /// §1): [`Output::Pairs`] of (left row, right row), in left row order and, within a left row, in right row
    /// order. Every side's columns have the same length.
    ///
    /// `by` is how a device finds the pairs; the pairs are the same either way. `right_id`, when set, names the
    /// right's key columns in the executor's space: a lane keeps what it built from them (the GPU its sort, or its
    /// grouping by document; the cores their sort) until the caller forgets the id ([`crate::exec::Executor::forget`]).
    /// One id always names the same columns: a lane refuses a kept entry whose row count differs.
    Join { left: Keys, right: Keys, right_id: Option<u64>, by: JoinBy },

    /// **Program.** A kernel the caller made ([`Program`]): [`Output::Program`].
    Program(ProgramKernel),
}

/// A relation's key columns, one per key, of equal length: each the host's, or staged (written after the launch
/// was begun, [`crate::col`]).
pub type Keys = Arc<[crate::col::Col]>;

/// How a join's pairs are found on a device (decision 0095 §4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum JoinBy {
    /// The right sorted by key 0 and kept; each left row searches its key's run.
    Probe,
    /// Both sides grouped by key 0's chunk (`v >> 16`, a document's nodes), each chunk's rows compared by brute
    /// force: for keys a sort cannot find quickly, at most three.
    Tile,
}

impl Kernel {
    /// The kernel's name, for prices and the profile.
    pub fn form(&self) -> Form {
        match self {
            Kernel::BitProduct { .. } => Form::BitProduct,
            Kernel::CountTable { emit: false, .. } => Form::CountTable,
            Kernel::CountTable { emit: true, .. } => Form::CountPairs,
            Kernel::Histogram { .. } => Form::Histogram,
            Kernel::Closure { .. } => Form::Closure,
            Kernel::Join { by: JoinBy::Probe, .. } => Form::Probe,
            Kernel::Join { by: JoinBy::Tile, .. } => Form::Tile,
            Kernel::Program(_) => Form::Program,
        }
    }

    /// The kernel's size in the unit its prices are per ([`crate::price`]):
    ///
    /// - bit product: bit operations, `a.rows × b.rows × words × 64`;
    /// - count table: rows read plus pairs tested, `left + right + Σ_p |left_p| × |right_p|` when emitting, else
    ///   `left + right`;
    /// - histogram: codes read;
    /// - closure: edges times the words of a bit row, `|dst| × ⌈n / 64⌉`;
    /// - join: rows of both sides (the pairs are not known ahead).
    pub fn units(&self) -> f64 {
        match self {
            Kernel::BitProduct { a, b, .. } => a.rows as f64 * b.rows as f64 * a.words as f64 * 64.0,
            Kernel::CountTable { loff, left, roff, right, emit } => {
                let base = left.len() as f64 + right.len() as f64;
                if !emit {
                    return base;
                }
                let pairs: f64 = (0..loff.len().saturating_sub(1))
                    .map(|p| (loff[p + 1] - loff[p]) as f64 * (roff[p + 1] - roff[p]) as f64)
                    .sum();
                base + pairs
            }
            Kernel::Histogram { codes, .. } => codes.len() as f64,
            Kernel::Closure { n, dst, .. } => dst.len() as f64 * (*n as f64 / 64.0).ceil().max(1.0),
            Kernel::Join { left, right, .. } => (left.first().map_or(0, |c| c.len()) + right.first().map_or(0, |c| c.len())) as f64,
            Kernel::Program(p) => p.0.units(),
        }
    }

    /// A join's shape is sound: as many keys on each side, at least one, each side's columns of one length.
    pub fn join_shape(left: &Keys, right: &Keys) -> Result<(usize, usize), String> {
        let (Some(l0), Some(r0)) = (left.first(), right.first()) else { return Err("join: no keys".into()) };
        if left.len() != right.len() {
            return Err(format!("join: {} left keys against {} right", left.len(), right.len()));
        }
        if left.iter().any(|c| c.len() != l0.len()) || right.iter().any(|c| c.len() != r0.len()) {
            return Err("join: a side's key columns differ in length".into());
        }
        if l0.len() > u32::MAX as usize || r0.len() > u32::MAX as usize {
            return Err("join: more rows than a u32 counts".into());
        }
        Ok((l0.len(), r0.len()))
    }
}

/// The physical forms, as prices and the profile name them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Form {
    BitProduct,
    /// A count table's counts per row.
    CountTable,
    /// A count table's pairs.
    CountPairs,
    Histogram,
    Closure,
    /// A join by sort and probe.
    Probe,
    /// A join by document tiles.
    Tile,
    /// A program of the caller's ([`Program`]): priced by itself, not by the table.
    Program,
}

impl Form {
    pub const ALL: [Form; 7] = [Form::BitProduct, Form::CountTable, Form::CountPairs, Form::Histogram, Form::Closure, Form::Probe, Form::Tile];

    pub fn name(self) -> &'static str {
        match self {
            Form::BitProduct => "bit product",
            Form::CountTable => "count table",
            Form::CountPairs => "count table (pairs)",
            Form::Histogram => "histogram",
            Form::Closure => "closure",
            Form::Probe => "join (sort and probe)",
            Form::Tile => "join (tiles)",
            Form::Program => "program",
        }
    }
}

/// A kernel's result.
///
/// Two outputs are equal when they hold the same values: pairs read in place ([`Output::View`]) equal the same
/// pairs copied ([`Output::Pairs`]).
#[derive(Clone, Debug)]
pub enum Output {
    /// A matrix of counts, row-major.
    Counts { rows: usize, cols: usize, data: Vec<u32> },
    /// One count per input row.
    PerRow(Vec<u32>),
    /// Pairs of row indexes, in the order the kernel defines.
    Pairs(Vec<(u32, u32)>),
    /// The same, read in place in a device's buffer: no copy.
    View(PairView),
    /// Rows of bits.
    Bits(Bits),
    /// A program's result ([`Kernel::Program`]).
    Program(ProgramResult),
}

/// A program's result in an [`Output`]: equal only to itself.
#[derive(Clone)]
pub struct ProgramResult(pub ProgramOutput);

impl std::fmt::Debug for ProgramResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProgramResult")
    }
}

impl Output {
    /// The pairs of a join or a pair-emitting kernel, owned or in place.
    pub fn into_pairs(self) -> Option<Pairs> {
        match self {
            Output::Pairs(p) => Some(Pairs::Owned(p)),
            Output::View(v) => Some(Pairs::View(v)),
            _ => None,
        }
    }

    /// The pairs, copied (tests, reports).
    pub fn pairs(&self) -> Option<Vec<(u32, u32)>> {
        match self {
            Output::Pairs(p) => Some(p.clone()),
            Output::View(v) => Some(v.iter().collect()),
            _ => None,
        }
    }
}

impl PartialEq for Output {
    fn eq(&self, other: &Output) -> bool {
        match (self, other) {
            (Output::Counts { rows: a, cols: b, data: c }, Output::Counts { rows: x, cols: y, data: z }) => a == x && b == y && c == z,
            (Output::PerRow(a), Output::PerRow(b)) => a == b,
            (Output::Bits(a), Output::Bits(b)) => a == b,
            (Output::Program(a), Output::Program(b)) => Arc::ptr_eq(&a.0, &b.0),
            (Output::Pairs(a), Output::Pairs(b)) => a == b,
            (Output::Pairs(_) | Output::View(_), Output::Pairs(_) | Output::View(_)) => {
                let (a, b) = (self.clone().into_pairs().expect("pairs"), other.clone().into_pairs().expect("pairs"));
                a.len() == b.len() && a.iter().eq(b.iter())
            }
            _ => false,
        }
    }
}

impl Eq for Output {}

/// Pairs read in place: segments of a device's buffer, in order, the buffer kept as long as the view (decision
/// 0100 §9). A probe's pairs are its threadgroups' blocks of the batch's arena, block by block in left row order.
#[derive(Clone)]
pub struct PairView {
    /// What keeps the memory: the buffer's lease.
    _keep: std::sync::Arc<dyn std::any::Any + Send + Sync>,
    base: *const [u32; 2],
    /// (first pair, pairs) of each segment.
    segs: std::sync::Arc<[(u32, u32)]>,
    len: usize,
}

// SAFETY: the memory is the kept buffer's, written by the device before the view was made and only read after.
unsafe impl Send for PairView {}
unsafe impl Sync for PairView {}

impl PairView {
    /// A view of `segs` of the pairs at `base`, whose memory `keep` holds.
    ///
    /// # Safety
    /// Every segment is inside the memory `keep` holds, written and not written again while the view lives.
    pub unsafe fn new(keep: std::sync::Arc<dyn std::any::Any + Send + Sync>, base: *const [u32; 2], segs: Vec<(u32, u32)>) -> PairView {
        let len = segs.iter().map(|&(_, n)| n as usize).sum();
        PairView { _keep: keep, base, segs: segs.into(), len }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The segments, as slices.
    pub fn segments(&self) -> impl Iterator<Item = &[[u32; 2]]> + '_ {
        // SAFETY: each segment is inside the kept memory (`new`'s contract).
        self.segs.iter().map(|&(at, n)| unsafe { std::slice::from_raw_parts(self.base.add(at as usize), n as usize) })
    }

    pub fn iter(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        self.segments().flatten().map(|p| (p[0], p[1]))
    }
}

impl std::fmt::Debug for PairView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

/// A join's pairs: owned, or read in place.
#[derive(Clone, Debug)]
pub enum Pairs {
    Owned(Vec<(u32, u32)>),
    View(PairView),
}

impl Pairs {
    pub fn len(&self) -> usize {
        match self {
            Pairs::Owned(v) => v.len(),
            Pairs::View(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn iter(&self) -> PairsIter<'_> {
        match self {
            Pairs::Owned(v) => PairsIter { cur: &[], owned: v.iter(), segs: None },
            Pairs::View(v) => PairsIter { cur: &[], owned: [].iter(), segs: Some(Box::new(v.segments())) },
        }
    }
}

/// The pairs of [`Pairs::iter`], in order.
pub struct PairsIter<'a> {
    owned: std::slice::Iter<'a, (u32, u32)>,
    cur: &'a [[u32; 2]],
    segs: Option<Box<dyn Iterator<Item = &'a [[u32; 2]]> + 'a>>,
}

impl Iterator for PairsIter<'_> {
    type Item = (u32, u32);
    #[inline]
    fn next(&mut self) -> Option<(u32, u32)> {
        if let Some(&p) = self.owned.next() {
            return Some(p);
        }
        loop {
            if let Some((p, rest)) = self.cur.split_first() {
                self.cur = rest;
                return Some((p[0], p[1]));
            }
            self.cur = self.segs.as_mut()?.next()?;
        }
    }
}
