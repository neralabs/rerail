//! The typed SQL shape the findings layer lowers to (S's `query/plan/sql_shape.rs`), and S's
//! hand-built phase-one emission over it (`generated_shape.rs`).
pub(in crate::code_rule_dsl) mod sql_shape;
pub(in crate::code_rule_dsl) mod generated_shape;
