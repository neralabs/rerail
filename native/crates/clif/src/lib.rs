//! Code written once in Cranelift's IR, for every backend that runs it (decision 0310).
//!
//! - [`view`]: a document's tape as a reader is told where it lies (`TapeView`, the device's
//!   `DocView`): offsets from the tape's base, its format and its widths.
//! - [`reads`]: the tape's primitive reads, format 15's and format 16's, as CLIF functions built
//!   once (a node's code number, its subtree size, its start and end, its parent, hops over its
//!   children and siblings), each marked hot or cold.
//! - [`rust`]: CLIF as Rust source: the parser's for `rr_aot`, and the reads' for the tape crate.
//! - [`msl`]: CLIF as Metal Shading Language: the reads' for the kernels.
//! - [`inline`]: the JIT's one inlining pass: a call of a hot read is replaced by its body, a cold
//!   one stays a call of the read compiled once in the module.
#[cfg(feature = "ir")]
pub mod inline;
#[cfg(feature = "jit")]
pub mod jit;
#[cfg(feature = "ir")]
pub mod reads;
#[cfg(feature = "ir")]
pub mod msl;
#[cfg(feature = "ir")]
pub mod rust;
pub mod view;
