//! The parsers' tests that need neither the engine nor Metal, in a crate of their own so that
//! they rebuild with the compiler crate alone: the `.rerail` ladder, checked against the engine's
//! own `.rerail` parser (`rerail_rr_syntax`), the JSON suite, checked against tree-sitter-json, and
//! both on Metal against the CPU (`metal_tests`, the Metal crate's block backend).
//! The helpers the engine's Metal measurements share are public (`json`).

#[cfg(test)]
mod aot_tests;
#[cfg(test)]
mod block_tests;
pub mod executors;
#[cfg(test)]
mod compile_tests;
#[cfg(test)]
mod corpus_tests;
pub mod json;
#[cfg(test)]
mod json_tests;
#[cfg(all(test, any(target_os = "macos", target_os = "linux", windows)))]
mod metal_tests;
#[cfg(test)]
mod rr_tests;
