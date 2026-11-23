//! A source's rows where they lie (a held relation's in its block), digested and lent to a keep as they are
//! (`KeptPicked`): what a run keeps of a stratum's rows or an input's, never gathered (decision 0353). The planner's,
//! which keeps a component's relations (`plan.rs`, `compose::retained`); the corpus's keeps read it (`retain`).
use crate::code_rule_dsl::facts::vtab::Source;
use crate::code_rule_dsl::logic::plan::Cells;
use rerail_blocks::vector::Vector;
use std::sync::Arc;

/// An order-independent digest of `source`'s rows (a set: rows are distinct), with its row count, of their content
/// (`Cells`, decision 0212): the same in every process, whichever ids it gave their atoms.
pub fn digest(source: &dyn Source, cells: &Cells) -> u64 {
    let mut sum: u64 = 0;
    each_row(source, cells, |_, h| sum = sum.wrapping_add(h));
    sum ^ (source.len() as u64).wrapping_mul(0xff51_afd7_ed55_8ccd)
}

/// Each of `source`'s rows as it holds it, with its digest of its content (`Cells::row`), read and keyed a chunk at
/// a time: one read of the atoms a chunk.
pub fn each_row(source: &dyn Source, cells: &Cells, mut f: impl FnMut(&[i64], u64)) {
    let width = source.columns().len();
    if width == 0 {
        return;
    }
    const CHUNK: usize = 4096;
    let mut keyed = Vec::with_capacity(width * CHUNK);
    // A held relation's rows, read where they lie, part by part (in its order, as `row_at` gives them).
    if crate::code_rule_dsl::logic::native::component::each_held_batch(source, width, |batch| {
        keyed.clear();
        keyed.extend_from_slice(batch);
        cells.key(&mut keyed);
        for (raw, key) in batch.chunks_exact(width).zip(keyed.chunks_exact(width)) {
            f(raw, crate::code_rule_dsl::logic::plan::row_digest(key));
        }
    })
    .is_some()
    {
        return;
    }
    let all: Vector<usize> = (0..width).collect();
    let mut raw = vec![0i64; width * CHUNK];
    let n = source.len();
    for start in (0..n).step_by(CHUNK) {
        let m = (n - start).min(CHUNK);
        for r in 0..m {
            source.row_at(start + r, &all, &mut raw[r * width..(r + 1) * width]);
        }
        keyed.clear();
        keyed.extend_from_slice(&raw[..m * width]);
        cells.key(&mut keyed);
        for r in 0..m {
            f(&raw[r * width..(r + 1) * width], crate::code_rule_dsl::logic::plan::row_digest(&keyed[r * width..(r + 1) * width]));
        }
    }
}

/// A source's rows read where they lie (a held relation's from its block, a batch at a time; another's by value), in
/// canonical order: a stratum's rows or an input's for an entry of their own, never gathered (`kept_strata`).
pub struct SourceRows {
    pub source: Arc<dyn Source>,
}

impl crate::content_store::kept_facts::KeptPicked for SourceRows {
    fn rows(&self) -> usize {
        self.source.len()
    }
    fn width(&self) -> usize {
        self.source.columns().len()
    }
    fn given(&self) -> bool {
        false
    }
    fn fill(&self, cols: &mut [i64], _: Option<&mut [u32]>) {
        let (n, k) = (self.rows(), self.width());
        let mut r = 0;
        let held = crate::code_rule_dsl::logic::native::component::each_held_batch(self.source.as_ref(), k, &mut |batch: &[i64]| {
            for row in batch.chunks_exact(k.max(1)) {
                for (c, &v) in row.iter().enumerate() {
                    cols[c * n + r] = v;
                }
                r += 1;
            }
        });
        if held.is_none() {
            for c in 0..k {
                for (i, v) in cols[c * n..(c + 1) * n].iter_mut().enumerate() {
                    *v = self.source.value(i, c);
                }
            }
        }
    }
}
