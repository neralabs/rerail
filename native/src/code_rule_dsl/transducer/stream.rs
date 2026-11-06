//! The stream program: Bitlex lexers over 64-byte blocks, in simdjson's structure.
//!
//! The tree under `stream/` copies simdjson's, file for file
//! (`docs/parser-generators/09-compile-and-lower.md` §12.2). The structural guards in `guards.rs`
//! check that layout, the API against simdjson's, and where loops, branches, indexing and
//! intrinsics may appear (§12.3).

#[cfg(test)]
mod guards;

// simdjson's `include/simdjson/arm64.h`: the arm64 implementation's headers.
#[cfg(target_arch = "aarch64")]
pub(in crate::code_rule_dsl) mod arm64;
// simdjson's `include/simdjson/internal/`: tables shared by the implementations.
pub(in crate::code_rule_dsl) mod internal;
