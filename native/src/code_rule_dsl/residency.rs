//! Residency (docs/final-shape/03-architecture.md §5): what a batch holds in memory, measured
//! against the 500 MB budget.
pub(crate) mod admission;
pub(in crate::code_rule_dsl) use admission::relational::{ast_stage::install_ast_producer, python_ast::PythonAst};
#[cfg(target_os = "macos")]
pub(crate) mod metal;
#[cfg(target_os = "macos")]
pub(crate) mod slabs;
