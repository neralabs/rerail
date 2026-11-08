//! A reference evaluator for Bitlex, the lexer language of docs/parser-generators/03-bitlex.md.
//!
//! Masks are `Vec<u64>`, one bit per byte, computed a 64-byte word at a time. Operations that
//! carry a state across words (fill, toggle, escapes) pass it word to word, as the block lowering
//! does. Scans, counts, machines and run operations are evaluated byte by byte: they give the
//! semantics, not the fast form.
pub mod eval;
pub mod mask;
pub mod oracle;
pub mod syntax;
pub mod tree;
