//! The gate's seeded mutants (`scripts/check/facts-gate.py`, `MUTANTS`): whether this process runs
//! mutant `n` (`FACTS_MUTANT=<n>`, read once; unset or 0 runs none). Compiled in only with the
//! `mutants` feature; without it `on` is a constant `false`. Mutants target rewrite kinds, one per
//! rewrite (`docs/facts/REWRITES.md` once it exists); until then the A3 views stand for the three
//! kinds, each caught by `FACTS_A3_CHECK` or the `FACTS_A3_OFF` reference:
//!
//! 1. dropped side condition: `subst.rs::edges` keeps rows whose `edge_parent` is NULL.
//! 2. lost union arm: `subst.rs::nodes` drops the last arm of its source union.
//! 3. wrong interval bound: `ast.rs::has_end_memo` starts the descendant range at `id + 2`
//!    (the first descendant is skipped).
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
