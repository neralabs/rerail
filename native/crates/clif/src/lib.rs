//! Code written once in Cranelift's IR, for every backend that runs it (decision 0310).
//!
//! - [`view`]: a document's tape as a reader is told where it lies (`TapeView`, the device's
//!   `ChunkTape` past its `at`): offsets from the tape's base, its format and its widths.
//! - [`reads`]: the tape's primitive reads, format 15's and format 16's, as CLIF functions built
//!   once (a node's code number, its subtree size, its start and end, its parent, hops over its
//!   children and siblings), each marked hot or cold.
//! - [`rust`]: CLIF as Rust source: the parser's for `rr_aot`, and the reads' for the tape crate.
//! - [`msl`]: CLIF as Metal Shading Language: the reads' for the kernels.
//! - [`relation`]: a relation's reads (decision 0325), the second family: a value of `rerail_blocks`' rows, a
//!   dead bit, an index's group, each read through the block's self-relative references.
//! - [`inline`]: the JIT's one inlining pass: a call of a hot read is replaced by its body, a cold
//!   one stays a call of the read compiled once in the module.
#[cfg(feature = "ir")]
pub mod inline;
// The browser build has no JIT (`wasm/README.md`), so no JIT of the reads either: the manifest
// leaves cranelift-jit out there.
#[cfg(all(feature = "ir", not(target_family = "wasm")))]
pub mod jit;
#[cfg(feature = "ir")]
pub mod reads;
#[cfg(feature = "ir")]
pub mod relation;
#[cfg(feature = "ir")]
pub mod msl;
#[cfg(feature = "ir")]
pub mod rust;
pub mod view;
