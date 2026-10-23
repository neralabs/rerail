//! The gate's seeded mutants (`scripts/check/facts-gate.py`, `MUTANTS`): whether this process runs
//! mutant `n` (`FACTS_MUTANT=<n>`, read once; unset or 0 runs none). Compiled in only with the
//! `mutants` feature; without it `on` is a constant `false`. Mutants target rewrite kinds, one per
//! rewrite (`docs/facts/REWRITES.md`). 1–3 seed the physical matcher (R12/R13, `lowered.rs`) and
//! are caught by the corpus differential, default plan vs `FACTS_REWRITES=0`:
//!
//! 1. dropped side condition: a stop rule (`stopBy: <rule>`) is dropped from its axis.
//! 2. lost union arm: an `OR`'s last disjunct is dropped.
//! 3. wrong interval bound: the descendant range starts at `id + 2` (the first descendant is
//!    skipped).
//!
//! Outside the rewrite kinds:
//!
//! 22. SQL derived-table scope: `sql/engine.rs::Planner::source_schema` plans a FROM subquery with
//!     the enclosing select's partial scope visible, so its outer references are one level too
//!     deep (read NULL) and FROM siblings resolve. Killed by
//!     `sql::engine::tests::derived_table_correlation_follows_sqlite_scoping`.

#[cfg(feature = "mutants")]
pub fn on(n: u8) -> bool {
    static MUTANT: std::sync::LazyLock<u8> =
        std::sync::LazyLock::new(|| std::env::var("FACTS_MUTANT").ok().and_then(|v| v.trim().parse().ok()).unwrap_or(0));
    *MUTANT == n
}

#[cfg(not(feature = "mutants"))]
#[inline(always)]
pub const fn on(_: u8) -> bool {
    false
}
