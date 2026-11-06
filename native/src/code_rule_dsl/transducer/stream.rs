//! Stream transducers (`docs/parser-generators/10-bit-programs.md`): the second program kind beside
//! the tape transducers (`01` §4, §8; `09` §7). A stream program reads a byte stream, many files
//! end to end, and computes 64-bit masks, one word per 64-byte block, by straight-line bit
//! operations with typed carries between blocks.
//!
//! This module is step 1 of `10` §5: the program, its verifier, its text form, and the word kernel
//! of every operation with its carry (a summary, an associative combine, an apply; `06` §2). The
//! interpreter (step 2), the lowering from Bitlex (step 3), the native IR (step 5), Metal (step 6)
//! and the extract pass (step 7) are not here.
//!
//! - `program`: the operations, their types, the program and its digest.
//! - `verify`: the rules every program is built through.
//! - `text`: the printer and the parser; `parse(print(p))` is `p`.
//! - `words`: one word kernel per mask operation, with its carry.
//! - `planes`: the scans over bit-planes (a count mod `n`, a clamped count).

pub(in crate::code_rule_dsl) mod planes;
pub(in crate::code_rule_dsl) mod program;
pub(in crate::code_rule_dsl) mod text;
pub(in crate::code_rule_dsl) mod verify;
pub(in crate::code_rule_dsl) mod words;

#[cfg(test)]
mod simdjson;
#[cfg(test)]
mod tests;
