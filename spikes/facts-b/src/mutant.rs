//! Seeded mutants for the gate (G2.6). With the `mutants` cargo feature, `on(n)` is true when the
//! environment variable `FACTS_MUTANT` equals `n`; without the feature it is a constant `false`,
//! so release builds carry no mutant code paths.
//!
//! Mutants owned here (the delta rules):
//! - 4: join drops the (w, w) pair (the ΔR⋈ΔS term of the bilinear expansion).
//! - 5: presence / count reduce ignores negative deltas (retractions not counted).
//! - 6: recursion stops after one step: records a fixpoint step derives at round ≥ 2 are dropped.
//! - 9: runtime tie detection off.
//! - 24: R15 filter pushdown moves a right-only WHERE conjunct below a LEFT join.
//! - 28, 30, …, 40, 44: R20–R34 (REWRITES.md, facts-b table), each at its rewrite in `gen/norm.rs`.
//! - 42: ordered aggregates fold a group's rows in representation order (`gen/lower.rs`).
//! - 46: a LEFT JOIN on equalities finds its unmatched rows by a semijoin (`gen/norm.rs` join).
//! - 50, 52, 54: R37 cross-plan sharing: the export map ignored (`gen/norm.rs` `Plans::add`), the
//!   export digest blind to inputs and scope (`gen/real.rs` `export_digest`), producers' failures
//!   not compared (`gen/real.rs` `producers_agree`).
//! - 56: the interner's epoch sweep does not mark the arrangements' rows (`gen/lower.rs` `walk_rows`).

#[cfg(feature = "mutants")]
pub fn on(n: u32) -> bool {
    static M: std::sync::LazyLock<Option<u32>> = std::sync::LazyLock::new(|| std::env::var("FACTS_MUTANT").ok().and_then(|v| v.parse().ok()));
    *M == Some(n)
}

#[cfg(not(feature = "mutants"))]
#[inline(always)]
pub fn on(_n: u32) -> bool {
    false
}
