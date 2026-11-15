//! Tape format 16 lab (decision 0156): every tradeoff (T1–T7) as code that can be measured.
//!
//! The flow, per captured file:
//!
//! ```text
//! capture (engine: source + format-15 image)
//!   -> v15::read            the tree as neutral nodes (kind, field, flags, supertypes, span)
//!   -> events::encode       format 16's open and close records          (1.1 of 0156)
//!   -> blocks::cut          content-defined blocks + placement           (T3)
//!   -> layout::lay          byte planes / field planes / records         (T2)
//!   -> codec                raw, zstd, LZ with output references, literal-pool LZ   (T1, T4)
//! and back: decode, positions and parents derived (T5), checked equal to the input tree.
//! ```
//!
//! `text` (T6) and `edits` (T7, E4) work from the same trees. `report` prints every metric.

pub mod base;
pub mod blocks;
pub mod capture;
pub mod codec;
pub mod edits;
pub mod huff;
pub mod inplace;
pub mod prim;
pub mod events;
pub mod layout;
pub mod lp;
pub mod measure;
pub mod relative;
pub mod repeat;
pub mod report;
pub mod solution;
pub mod span;
pub mod text;
pub mod tree;
pub mod v15;
pub mod writer;
#[cfg(target_os = "macos")]
pub mod gpu;
#[cfg(target_os = "macos")]
pub mod inplace_gpu;
#[cfg(target_os = "macos")]
pub mod prim_gpu;
#[cfg(target_os = "macos")]
pub mod scan;
#[cfg(target_os = "macos")]
pub mod columns;

