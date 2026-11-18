//! Setfix's rows: the rows' storage is the facts layer's (`facts::rows`, decision 0325), shared with the vtab's
//! `Relation`; here, how setfix's readers read them (`RowData`).

pub use crate::code_rule_dsl::facts::rows::*;

use super::runs::RowData;

impl RowData for Rows {
    #[inline(always)]
    fn row(&self, id: u32, k: usize) -> Row {
        Rows::row(self, id, k)
    }
    /// Row `id`'s value in column `c`: one read where it lies.
    #[inline(always)]
    fn value(&self, id: u32, _k: usize, c: usize) -> i64 {
        Rows::get(self, id, c)
    }
}

impl RowData for Vec<i64> {
    #[inline(always)]
    fn row(&self, id: u32, k: usize) -> Row {
        Row::of(&self[id as usize * k..][..k])
    }
    #[inline(always)]
    fn value(&self, id: u32, k: usize, c: usize) -> i64 {
        self[id as usize * k + c]
    }
}

/// The chunk scope's tests that need the engine (its jobs, its binding); the rest are `facts::rows`'s.
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn in_block_of(rows: &Rows) -> Arc<Facts> {
        rows.in_block().expect("rows").facts
    }

    fn some_rows() -> Rows {
        let mut r = Rows::new(2);
        r.extend_from_slice(&[1, 2, 3, 4]);
        r
    }

    /// A job the chunk starts (its strata's, on another thread) puts its relations in the chunk's blocks: the
    /// scope is the run's context, carried.
    #[test]
    fn a_job_of_the_chunk_puts_its_relations_in_the_chunks_block() {
        in_chunk(|| {
            let mine = Arc::as_ptr(&in_block_of(&some_rows())) as usize;
            let theirs = std::sync::Mutex::new(0usize);
            crate::threads::scope(|jobs| {
                let got = &theirs;
                jobs.spawn("a test job", move |_| *got.lock().unwrap() = Arc::as_ptr(&in_block_of(&some_rows())) as usize);
            });
            assert_eq!(mine, theirs.into_inner().unwrap(), "the job wrote in the chunk's block");
        });
    }

    /// A run bound to the CPU takes anonymous blocks (Q58): resident only where written, never a pinned buffer of
    /// the device's pool, which the run's footprint would count whole.
    #[test]
    fn a_cpu_run_takes_anonymous_blocks() {
        let was = crate::code_rule_dsl::logic::plan::placement::set_binding(crate::code_rule_dsl::logic::plan::placement::Binding::Cpu);
        let r = in_chunk(some_rows);
        crate::code_rule_dsl::logic::plan::placement::set_binding(was);
        assert!(!in_block_of(&r).block.in_slab(), "a CPU run's block is anonymous");
    }
}
