//! The set-based SQL engine for code rules (`docs/sql-set-composition-design.md`): combinators
//! lower to joins and set operations over frames in SQLite, and the leaves are native terminals
//! behind virtual tables, each run once per batch over its whole input set. The native matcher is
//! the production engine and the parity oracle; the evaluator selects this one at run time
//! (`eval::Engine::Sql`).
//!
//! # Documents of a batch
//!
//! Every row the engine reads or writes names its document by a number (the `file` column of the
//! `node` table, of `runs` and `roots`, of every frame and stage) and a node by its number in that
//! document. Two numbering schemes share the column, side by side:
//!
//! | | batch documents | parsed sub-documents |
//! |---|---|---|
//! | what | one document of a file (`embed`): its own tree, or a program it embeds (a Blade view's compiled PHP, a Vue component's scripts) | a tree `parse` (or `formatArgs`) reads from a string literal or comment of a batch document |
//! | held in | `exec::Batch::files` (`BatchFile`) | `exec::Batch::subfiles` (`SubFile`), with the batch document it was parsed from (`host`) |
//! | numbered | `0..files.len()`, in the order the driver passes them | from `files.len()` on, in the order the terminals parse them, lazily and once per tree per batch (`Batch::add_subfile`) |
//! | its nodes | the tape's preorder indexes, its node ids | the tree's preorder indexes; its node ids carry `literal::ID_FLAG` and the tree's base above them (`nodes::sub_index`, `terminals::fact::local_id`) |
//! | language | one per document: its dialect's grammar and kind codes | the literal grammar's |
//!
//! A batch document is a tape of one language (`tape::TapeDoc`): no tape holds another, so its
//! sibling walks never cross a language boundary, and a file that embeds a program is two batch
//! documents, each with its own dialect, syntax errors, scopes and anchor selection, read as the
//! native evaluator reads it (`eval::query::read_document`, `workspace::evaluate::DocumentRead`).
//! A statement's kind codes are its clause's dialect's: it runs on documents of that dialect only,
//! a stage on those of its own (`exec::Session::run_batch`), and the kind postings a `kind` filter
//! reads are each document's own (`nodes::NodeFacts`), so they are keyed by dialect. Nothing in SQL
//! joins two documents of a file: a rule crossing between them (`embedded`, a program's `host`) is
//! refused. A match's nodes stand for the bytes of the file the native renderer maps them to
//! through the document's source map (`origin::InFile`), as a native match's do.

pub(super) mod crossfile;
mod emit;
mod exec;
mod lower;
mod nodes;
mod plan;
mod recursion;
pub(super) mod select_when;
mod terminals;
#[cfg(test)]
mod tests;

#[cfg(test)]
pub(super) use exec::compile;
pub(super) use exec::{
    ClauseRun, Compiled, FileContext, Session, SqlError, SqlMatch, Stats, compile_each,
    compile_enumerating, environment,
};
#[cfg(feature = "unbounded-work")]
pub(super) use exec::{StatementProfile, profiled};
