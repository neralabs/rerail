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

/// A source's rows read where they lie, in canonical order, by the store's writer as it packs them (`KeptPicked`,
/// decision 0353 step 8): a stratum's rows or an input's for an entry of their own, never gathered (`kept_strata`).
/// A row's search starts from the last one's (`Source::value_memo`; any memo is correct, so threads share it).
pub struct SourceRows {
    pub source: Arc<dyn Source>,
    memo: std::sync::atomic::AtomicU32,
}

impl SourceRows {
    pub fn new(source: Arc<dyn Source>) -> SourceRows {
        SourceRows { source, memo: std::sync::atomic::AtomicU32::new(crate::code_rule_dsl::facts::vtab::NO_MEMO) }
    }
}

impl crate::content_store::kept_facts::KeptPicked for SourceRows {
    /// A relation's rows: sorted on column 0 alone (decision 0678 step 8).
    fn order(&self) -> rerail_blocks::packed::Order {
        rerail_blocks::packed::Order::Column
    }
}

impl rerail_blocks::packed::RowsIn for SourceRows {
    fn n(&self) -> usize {
        self.source.len()
    }
    fn arity(&self) -> usize {
        self.source.columns().len()
    }
    fn get(&self, r: usize, c: usize) -> i64 {
        use std::sync::atomic::Ordering::Relaxed;
        let was = self.memo.load(Relaxed);
        let mut memo = was;
        let x = self.source.value_memo(r, c, &mut memo);
        if memo != was {
            self.memo.store(memo, Relaxed);
        }
        x
    }
}

/// A relation's digest taken from its rows where they lie: the digest `rows_digest` takes of its live rows, read a
/// batch at a time in its block (decision 0353 step 8).
pub fn held_digest(cells: &Cells, held: &crate::code_rule_dsl::logic::native::component::InRel) -> Option<u64> {
    Some(cells.digest_batches(held.len(), held.rows(), &|lo, hi, f| {
        held.each_live_batch(lo, hi, |rows| f(rows));
    }))
}
