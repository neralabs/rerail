//! The set-based SQL engine for code rules (`docs/sql-set-composition-design.md`): combinators
//! lower to joins and set operations over frames in SQLite, and the leaves are native terminals
//! behind virtual tables, each run once per batch over its whole input set. The native matcher is
//! the production engine and the parity oracle; the evaluator selects this one at run time
//! (`eval::Engine::Sql`).

pub(super) mod crossfile;
mod emit;
mod exec;
mod lower;
mod nodes;
mod plan;
pub(super) mod select_when;
mod terminals;
#[cfg(test)]
mod tests;

pub(super) use exec::{SqlError, SqlMatch, compile, environment};
