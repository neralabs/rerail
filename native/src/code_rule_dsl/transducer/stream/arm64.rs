//! simdjson's arm64 implementation (`include/simdjson/arm64.h`): the primitive layer.

pub(in crate::code_rule_dsl) mod bitmanipulation;
pub(in crate::code_rule_dsl) mod bitmask;
pub(in crate::code_rule_dsl) mod simd;

#[cfg(test)]
mod simd_tests;
