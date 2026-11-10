//! The parsers' tests that need neither the engine nor Metal, in a crate of their own so that
//! they rebuild with the compiler crate alone: the `.rr` ladder, checked against the engine's
//! own `.rr` parser (`rerail_rr_syntax`), the JSON suite, checked against tree-sitter-json, and
//! both on Metal against the CPU (`metal_tests`, the Metal crate's block backend).
//! The helpers the engine's Metal measurements share are public (`json`).

pub mod json;
#[cfg(test)]
mod json_tests;
#[cfg(all(test, target_os = "macos"))]
mod metal_tests;
#[cfg(test)]
mod rr_tests;
