//! Typed SQL for the logic language's plans (05 Phase 5 row 4), from
//! ner-1562-scale-test-the-re-railer's `query/plan/render/{sql_ast, typed_boundary}.rs` and
//! `query/plan/physical.rs`: a SQL AST rendered and checked against the relations a statement may
//! read, so emitted SQL names only declared fact families. N's `relational_ir.rs`/`relational.rs`
//! (tree axes in SQL) are not kept: tree navigation is the transducer's.
pub(in crate::code_rule_dsl) mod physical;
pub(in crate::code_rule_dsl) mod sql_ast;
pub(in crate::code_rule_dsl) mod typed_boundary;
