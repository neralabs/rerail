//! Lattice facts (Moti-Velz/facts-engine `docs/facts/LATTICE.md`): E1's semiring-round fixpoint
//! on the CPU and E3's maintenance of it per edit, copied from facts-b's `lattice` module.
//!
//! - `round`: the operator (L5): CSR (flat or base + replacement rows), `set` and `min_level`; a
//!   sequential kernel and four parallel-within-a-round ones (push over frontier word ranges,
//!   push over edge ranges, owner-computes push, direction-optimizing push/pull); the naive
//!   reference. Depends on nothing else in the crate.
//! - `cols`: columns written once and mapped read-only (L6), with the copy counter; columns a
//!   worktree writes, mapped privately (copy-on-write pages).
//! - `maint` (E3): the layered CSR updated in place; `min_level` and `set` maintained per edit
//!   in the L5 shape. Depends only on `round`.
//!
//! facts-b's command-line drivers (`lattice build|run|maint`) are not here: E1's bench is not
//! copied, and E3's replay is the dead-code producer's workload test.
pub mod cols;
pub mod maint;
pub mod round;

/// facts-b's seeded mutants (`crate::mutant::on`, `FACTS_MUTANT` under the `mutants` feature),
/// as a test-only seam: a test turns one on for its own thread; outside tests it is `false`.
/// 75: `min_level`'s re-check counts an invalidated predecessor as a witness (`level_update`);
/// 77: `set`'s over-delete takes no forward round (`set_update`); 79: `set` by B/F accepts an id
/// reached before the edit as support (`set_update_bf`).
pub(crate) mod mutant {
    #[cfg(test)]
    thread_local! {
        static ON: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    }

    #[cfg(test)]
    pub(crate) fn on(n: u32) -> bool {
        ON.with(|m| m.get() == n)
    }

    #[cfg(not(test))]
    #[inline(always)]
    pub(crate) fn on(_n: u32) -> bool {
        false
    }

    /// Runs `f` with mutant `n` on in this thread.
    #[cfg(test)]
    pub(crate) fn with<T>(n: u32, f: impl FnOnce() -> T) -> T {
        struct Off;
        impl Drop for Off {
            fn drop(&mut self) {
                ON.with(|m| m.set(0));
            }
        }
        ON.with(|m| m.set(n));
        let _off = Off;
        f()
    }
}
