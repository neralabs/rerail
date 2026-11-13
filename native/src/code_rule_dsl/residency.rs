//! Residency (docs/final-shape/03-architecture.md §5): what a batch holds in memory, measured
//! against the 500 MB budget.
pub use rerail_w::code_rule_dsl::residency::admission;
pub use admission::relational::ast_stage::install_ast_producer;
// cratify: `metal` is rerail-device-metal's (it mounts it)
// cratify: `slabs` is rerail-device-metal's (it mounts it)
