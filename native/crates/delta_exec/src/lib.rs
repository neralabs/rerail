//! A delta executor that loads itself from one tape (decision 0686), built beside the engine to the agreed design
//! (*Bootstrap from one tape*, *Delta-first engine primitives*), milestone by milestone as the rebuild plan sets
//! them, each passing its checks before the next starts.
//!
//! It holds the pieces that are already the final shape:
//!
//! - `dir`: the directory (B1): block ids, references between generations (only to frozen blocks, each holding its
//!   referent), and a launch's pins and descriptor table;
//! - `op`: operations over blocks (B1): signatures of at most 30 slots, each placed as phases on the six axes; the
//!   registry of versions, each operation's twin (`rerail_delta_twins`) its reference and every other version
//!   registered only once checked against it; the CPU executor, which cuts each phase's items into tiles;
//! - `trace`: the program trace (B2), on the host as handles only: a chain of frozen layer blocks, each written by
//!   `consolidate` (or rolled up by `merge`) into a block of its own over the one below; its two states and the
//!   change between them read where the layers lie (`rerail_blocks::relation::View::chain`, `Bistate`);
//! - `sym`: the symbol table, the one program global;
//! - `tape`: a document's text and its views (`kind`, `field`, `child`, `text`, `value`, `doc`), read where the
//!   tape lies;
//! - `kform`: K-form's relations;
//! - `k0`: L1's walk of a K0 tape into K-form rows.
//!
//! Files are not read here: loads are answered through the engine's file layer (the observer, its placed read into a
//! residency slab, content-addressed sources and tapes), as *0686: Operations* §9 sets out. `Text` is a source given
//! in memory (the bootstrap's own files and tests).
//!
//! The milestones toward one boot operation, L1 (the rebuild plan's §4), come after: the operator contract (B3), l0's
//! tape through the file layer (B4), symbols (B5) and L1 itself (B6).

pub mod cache;
pub mod contract;
pub mod dir;
pub mod k0;
pub mod kform;
pub mod op;
pub mod sym;
pub mod tape;
pub mod trace;

#[cfg(test)]
mod tests;

use std::sync::Arc;
use tape::Text;

/// `bootstrap-l0.rerail`: the reader of rules (P2) and the rule that orders the bootstrap's modules.
pub const L0: &str = include_str!("../rerail/bootstrap-l0.rerail");

/// A text parsed, for callers holding a source.
pub fn text(src: &str) -> Arc<Text> {
    // copy: load (a source's bytes)
    Arc::new(Text::parse(src.to_string()).expect("an rr file that parses"))
}
