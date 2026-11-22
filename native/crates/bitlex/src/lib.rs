//! The Bitlex compiler, on the compiler's front end (`rerail-compiler`): a grammar's masks and tokens to tier 1's
//! block program (`bitlex_masks`), the provers that decide its questions (`bitlex_prove`, `bitlex_sat`, their answers
//! kept on the machine's disk: `answers`), and its tree section to tier 2's words (`bitlex_tier2`). A grammar's Rust is
//! written here too, from the grammar: tier 1 (`bitlex_lex_rust`), tier 2 (`bitlex_rust`). The sources stay in the
//! compiler's folder (`crates/compiler/src`); this crate mounts them.

// The compiler's passes hand each other plain tuples of tables; naming each would add nothing.
#![allow(clippy::type_complexity)]

// The crates below under their modules' names, so the mounted sources' `crate::records`, `crate::block`, ... read them.
#[allow(unused_imports)]
use rerail_compiler::*;

/// The provers' answers kept on the machine's file system (decision 0414): no file system on wasm.
#[cfg(not(target_family = "wasm"))]
#[path = "../../compiler/src/answers.rs"]
pub mod answers;
/// Tier 1 as Rust, written from the lexer's statements (decision 0550).
#[path = "../../compiler/src/bitlex_lex_rust.rs"]
pub mod bitlex_lex_rust;
#[path = "../../compiler/src/bitlex_masks.rs"]
pub mod bitlex_masks;
#[path = "../../compiler/src/bitlex_pair_lookup.rs"]
pub mod bitlex_pair_lookup;
#[path = "../../compiler/src/bitlex_piece_checks.rs"]
pub mod bitlex_piece_checks;
#[path = "../../compiler/src/bitlex_prove.rs"]
pub mod bitlex_prove;
/// Tier 2 as Rust, written from the tree rules (decision 0550).
#[path = "../../compiler/src/bitlex_rust.rs"]
pub mod bitlex_rust;
#[path = "../../compiler/src/bitlex_sat.rs"]
pub mod bitlex_sat;
#[path = "../../compiler/src/bitlex_tier2.rs"]
pub mod bitlex_tier2;
