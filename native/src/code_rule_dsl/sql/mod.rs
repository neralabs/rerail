//! The set-based SQL engine for code rules (`docs/sql-set-composition-design.md`): combinators
//! lower to joins and set operations over frames in SQLite, and the leaves are native terminals
//! behind virtual tables, each run once per batch over its whole input set. The native matcher is
//! the production engine and the parity oracle; the evaluator selects this one at run time
//! (`eval::Engine::Sql`).

pub(super) mod crossfile;
mod emit;
mod exec;
mod frames;
mod lower;
mod nearest;
mod nodes;
mod plan;
mod recursion;
pub(super) mod select_when;
mod shapes;
mod skip;
mod terminals;
#[cfg(test)]
mod tests;
mod texts;
mod timing;

pub(super) use exec::{
    ClauseRun, CompilePhases, Compiled, FileContext, ProgramCompiler, Session, SqlError, SqlMatch,
    Stats, compile_enumerating, environment,
};
#[cfg(feature = "unbounded-work")]
pub(super) use exec::{StatementProfile, profiled};
#[cfg(test)]
pub(super) use exec::{compile, compile_each};
#[cfg(test)]
pub(super) use nodes::POSTINGS_TIME;
#[cfg(test)]
pub(super) use shapes::VERIFY as VERIFY_SHAPES;
#[cfg(test)]
pub(super) use timing::Started;
pub(super) use timing::{Timing, timed};

/// Files per batch where the caller sets none: the workspace's default, and every batch of a
/// stateless evaluation.
pub(super) const DEFAULT_BATCH_SIZE: usize = 64;

/// The mutation kit's switch: whether this process runs seeded semantic mutant `n`
/// (`RERAIL_SQL_MUTANT=<n>`, read once; unset or 0 runs none). Each mutant is a plausible
/// regression of one hard construct, a few lines at its site, that the differential and the
/// corpus census must catch:
///
/// 1. a binding relation takes any match, not the argmin per `k` (`emit`);
/// 2. `any` runs its branches last first, so a later branch decides where an earlier one matched (`emit`);
/// 3. `not`'s probe ignores the captures bound before it (`lower`);
/// 4. a three-valued `all` withholds no term (`emit/three`);
/// 5. a strict three-valued search keeps searching past an unknown candidate (`emit/three`);
/// 6. a three-valued `count` reads an unknown candidate as false (`emit/three`);
/// 7. `nthChild` counts anonymous siblings (`nodes`);
/// 8. `has` with `stopBy: neighbor` skips anonymous children (`emit`);
/// 9. a `stopBy` rule's stop node is not tested (`emit`);
/// 10. `violation.each` reports a repeated range instead of stopping there (`emit`);
/// 11. a recursion loop drops its last pass (`exec`);
/// 12. a call returns its parameters where they conflict (`emit`);
/// 13. distinct-key evaluation omits one read slot from the key (`emit/keyed`);
/// 14. `constraints` run in reverse order (`lower`);
/// 15. a `field` relation ignores the field (`emit`);
/// 16. the skip takes an anti-join whose probes are empty for an empty output (`skip`);
/// 17. the pattern terminals' tested nodes are keyed by node only, not (term, node) (`exec`);
/// 18. a unit's key drops the caller's captures it reads (`lower`);
/// 19. a stage `ofRule`'s `nthChild` ranks over the whole stage, not per parent (`emit`,
///     `emit/three`);
/// 20. a binding's site, value or annotation sub-rule ignores the `FOLLOWING` guard, entering a
///     binding node again under the sub-rule it is matching (`emit/following`).
/// 21. a call's unit drops from its key the parameters its body reads and never holds, so it
///     reads them unbound (`lower`).
///
/// `tests/replay.jsonl` (`replayed_cases_agree`) kills each of them; `just rerail-sql-gate
/// --mutants` checks it does, and that the differential at the gate's case count does too.
///
/// Only test builds and the never-shipped `sql-mutants` feature read the switch; elsewhere it is
/// the constant `false`, so production code is unaffected.
#[cfg(any(test, feature = "sql-mutants"))]
fn mutant(n: u8) -> bool {
    static MUTANT: std::sync::LazyLock<u8> = std::sync::LazyLock::new(|| {
        std::env::var("RERAIL_SQL_MUTANT")
            .ok()
            .and_then(|value| value.trim().parse().ok())
            .unwrap_or(0)
    });
    *MUTANT == n
}

#[cfg(not(any(test, feature = "sql-mutants")))]
#[inline(always)]
const fn mutant(_: u8) -> bool {
    false
}
