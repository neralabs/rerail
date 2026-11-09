//! The kernels: each physical form of decision 0098 §1 as a function of plain columns, with one meaning.
//!
//! Inputs are shared (`Arc`), so a batch can hold them without copying and a lane can read them in place (the GPU
//! wraps them as no-copy buffers when they are page-aligned, and copies them otherwise).
//!
//! Partitions: several kernels work per partition (a document, a key run). A partitioned column comes with offsets,
//! `nparts + 1` of them, ascending, the first 0 and the last the column's length. Partition `p` is
//! `col[off[p]..off[p + 1]]`.

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
        }
    }

    /// The kernel's size in the unit its prices are per ([`crate::price`]):
    ///
    /// - bit product: bit operations, `a.rows × b.rows × words × 64`;
    /// - count table: rows read plus pairs tested, `left + right + Σ_p |left_p| × |right_p|` when emitting, else
    ///   `left + right`;
    /// - histogram: codes read;
    /// - closure: edges times the words of a bit row, `|dst| × ⌈n / 64⌉`.
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
        }
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
}

impl Form {
    pub const ALL: [Form; 5] = [Form::BitProduct, Form::CountTable, Form::CountPairs, Form::Histogram, Form::Closure];

    pub fn name(self) -> &'static str {
        match self {
            Form::BitProduct => "bit product",
            Form::CountTable => "count table",
            Form::CountPairs => "count table (pairs)",
            Form::Histogram => "histogram",
            Form::Closure => "closure",
        }
    }
}

/// A kernel's result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Output {
    /// A matrix of counts, row-major.
    Counts { rows: usize, cols: usize, data: Vec<u32> },
    /// One count per input row.
    PerRow(Vec<u32>),
    /// Pairs of row indexes, in the order the kernel defines.
    Pairs(Vec<(u32, u32)>),
    /// Rows of bits.
    Bits(Bits),
}
