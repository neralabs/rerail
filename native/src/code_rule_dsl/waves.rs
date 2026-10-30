//! SQL-driven waves (docs/final-shape/03-architecture.md §4, §7; decision 0010): one owner
//! connection on the content store, the temp schema's lifecycle, and the frontier loop that SQL
//! steers, wave by wave, until no relation changes.
mod owner;
/// D's relational stage as copied, test-only (05 Phase 7; open item O19).
#[cfg(test)]
#[allow(dead_code)]
pub(in crate::code_rule_dsl) mod relational;
/// The first slice's wave over lowered matchers (06 S4).
#[allow(dead_code)]
pub(in crate::code_rule_dsl) mod matches;
mod schedule;
mod storage;
