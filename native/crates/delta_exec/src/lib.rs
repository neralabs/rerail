//! A delta executor that loads itself from one tape (decision 0686), built beside the engine to the agreed design
//! (*Bootstrap from one tape*, *Delta-first engine primitives*), milestone by milestone as the rebuild plan sets
//! them, each passing its checks before the next starts.
//!
//! At M0 it holds the pieces that are already the final shape:
//!
//! - `sym`: the symbol table, the one program global;
//! - `tape`: a document's text and its views (`kind`, `field`, `child`, `text`, `value`, `doc`), read where the
//!   tape lies;
//! - `kform`: K-form's relations;
//! - `k0`: L1's walk of a K0 tape into K-form rows;
//! - `world`: what a `load` is answered from.
//!
//! The traces (M1), the operator contract (M2), the step operators (M3), the waves, fixpoints, answers and phases
//! (M4 to M7) and the loop (M8) come after.

pub mod k0;
pub mod kform;
pub mod sym;
pub mod tape;
pub mod world;

#[cfg(test)]
mod tests;

use std::sync::Arc;
use tape::Text;

/// `bootstrap-l0.rerail`: the reader of rules (P2) and the rule that orders the modules its loads ask for.
pub const L0: &str = include_str!("../rerail/bootstrap-l0.rerail");

/// A text parsed, for callers holding a source.
pub fn text(src: &str) -> Arc<Text> {
    // copy: load (a source's bytes)
    Arc::new(Text::parse(src.to_string()).expect("an rr file that parses"))
}
