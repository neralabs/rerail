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
//! Files are not read here: a document is answered through the engine's file layer (`files`: the observer, its
//! placed read into the placer's home, the rr parser's tape written into a tape block, a blob named by its content),
//! as *0686: Operations* §9 sets out. `text` makes a document of a source given in memory (tests).
//!
//! The milestones toward one boot operation, L1 (the rebuild plan's §4), come after: the operator contract (B3), l0's
//! tape through the file layer (B4), symbols (B5) and L1 itself (B6).

pub mod cache;
pub mod contract;
pub mod dir;
pub mod files;
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
    // copy: load (a source in hand: a test's literal, the bootstrap's own file)
    let home = rerail_rr_syntax::tape_format::home::Home::Own(rerail_rr_syntax::tape_format::Vector::from(src.as_bytes()));
    Arc::new(Text::parse(rerail_rr_syntax::tape_format::home::Text::new(home).expect("UTF-8")).expect("an rr file that parses"))
}
