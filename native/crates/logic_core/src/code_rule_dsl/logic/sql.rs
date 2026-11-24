//! Typed SQL for the logic language's plans (05 Phase 5 row 4), from
//! N's `query/plan/render/{sql_ast, typed_boundary}.rs` and
//! `query/plan/physical.rs`: a SQL AST rendered and checked against the relations a statement may
//! read, so emitted SQL names only declared fact families. N's `relational_ir.rs`/`relational.rs`
//! (tree axes in SQL) are not kept: tree navigation is the transducer's.
pub mod physical;
pub mod select;
pub mod sql_ast;
pub mod typed_boundary;

pub mod program;
pub mod properties;

// A relation's name in SQL, below the plans' SQL and the run (rerail-facts-base).
pub use super::sql_names::{ident, table, unquote};
