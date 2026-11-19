//! Code written once in Cranelift's IR, for every backend that runs it (decision 0310).
//!
//! - [`view`]: a document's tape as a reader is told where it lies (`TapeView`, the device's
//!   `DocView`): offsets from the tape's base, its format and its widths.
//! - [`reads`]: the tape's primitive reads, format 15's and format 16's, as CLIF functions built
//!   once (a node's code number, its subtree size, its start and end, its parent, hops over its
//!   children and siblings), each marked hot or cold.
//! - [`rust`]: CLIF as Rust source: the parser's for `rr_aot`, and the reads' for the tape crate.
//! - [`ukl`]: CLIF as UKL: the reads for every GPU's kernels (`<ukl/reads/*.h>`).
//! - [`relation`]: a relation's reads (decision 0325), the second family: a value of `rerail_blocks`' rows, a
//!   dead bit, an index's group, each read through the block's self-relative references.
//! - [`ids`]: a projection's numbering as its reads are told where it lies (decision 0353 step 2): its rank index
//!   and granule table, each a run of its block.
//! - [`id_reads`]: the numbering's reads (a node's or token's id from its number, and back; order keys; rank and
//!   select), in the relation's family.
//! - [`inline`]: the JIT's one inlining pass: a call of a hot read is replaced by its body, a cold
//!   one stays a call of the read compiled once in the module.
pub mod ids;
#[cfg(feature = "ir")]
pub mod id_reads;
#[cfg(feature = "ir")]
pub mod inline;
#[cfg(feature = "ir")]
pub mod jit;
#[cfg(feature = "ir")]
pub mod reads;
#[cfg(feature = "ir")]
pub mod relation;
#[cfg(feature = "ir")]
pub mod ukl;
#[cfg(feature = "ir")]
pub mod rust;
pub mod view;
pub mod view_reads;
