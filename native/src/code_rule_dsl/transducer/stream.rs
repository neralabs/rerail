//! The stream program: Bitlex lexers over 64-byte blocks, in simdjson's structure.
//!
//! The tree under `stream/` copies simdjson's, file for file
//! (`docs/parser-generators/09-compile-and-lower.md` §12.2). The structural guards in `guards.rs`
//! check that layout, the API against simdjson's, and where loops, branches, indexing and
//! intrinsics may appear (§12.3).

#[cfg(test)]
mod guards;
