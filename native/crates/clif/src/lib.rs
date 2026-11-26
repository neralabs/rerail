//! Code written once in Cranelift's IR, for every backend that runs it (decision 0310).
//!
//! - [`view`]: a document's tape as a reader is told where it lies (`TapeView`, the device's
//!   `DocView`): offsets from the tape's base, its format and its widths.
//! - [`reads`]: the tape's primitive reads, format 15's and format 16's, as CLIF functions built
//!   once (a node's code number, its subtree size, its start and end, its parent, hops over its
//!   children and siblings), each marked hot or cold.
//! - [`ukl`]: CLIF as UKL: the reads for every GPU's kernels (`<ukl/reads/*.h>`).
//! - [`relation`]: a relation's reads (decision 0325), the second family: a value of `rerail_blocks`' rows, a
//!   dead bit, an index's group, each read through the block's self-relative references.
//! - [`ids`]: a projection's numbering as its reads are told where it lies (decision 0353 step 2): its rank index
//!   and granule table, each a run of its block.
//! - [`id_reads`]: the numbering's reads (a node's or token's id from its number, and back; order keys; rank and
//!   select), in the relation's family.
//! - [`inline`]: the JIT's one inlining pass: a call of a hot read is replaced by its body, a cold
//!   one stays a call of the read compiled once in the module.
// The JIT by one name on every target: Cranelift's, or the tree's stand-in on WASM (`Cargo.toml`).
#[cfg(all(feature = "ir", target_family = "wasm"))]
extern crate cranelift_jit_wasm as cranelift_jit;
pub use rerail_reads::ids;
#[cfg(feature = "ir")]
pub mod id_reads;
#[cfg(feature = "ir")]
pub mod inline;
// The browser build has no JIT (`wasm/README.md`), so no JIT of the reads either: the manifest
// leaves cranelift-jit out there.
#[cfg(all(feature = "ir", not(target_family = "wasm")))]
pub mod jit;
/// The text tests' verdicts in the rolling cache (decision 0478 step 2d).
#[cfg(feature = "ir")]
pub mod verdict_reads;
/// The atoms' reads (decision 0478 step 2b).
#[cfg(feature = "ir")]
pub mod atoms_reads;
#[cfg(feature = "ir")]
pub mod reads;
#[cfg(feature = "ir")]
pub mod relation;
#[cfg(feature = "ir")]
pub mod ukl;
// The view and the numbering, with their host reads, in the crate below every reader of them (`rerail_reads`).
pub use rerail_reads::view;
/// The tape's writer in CLIF (decision 0434 §6): its state's layout, and (with `ir`) its ops.
pub mod writer;
/// The views' entries' layout always (the facts write them); their reads in CLIF with `ir`.
pub mod view_reads;
