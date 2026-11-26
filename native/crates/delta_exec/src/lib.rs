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
//! - `files`: a document through the engine's file layer (B4), its source and tape in its tape block, a blob;
//! - `sym`: the atoms block, the one program global, and `intern` (B5), its one writer: a changed document's
//!   identifiers and strings, with its node → symbol facts;
//! - `tape`: a document's text and its views (`kind`, `field`, `child`, `text`, `value`, `doc`), read where the
//!   tape and its symbols lie;
//! - `kform`: K-form's relations;
//! - `k0`: `k0_pass`, L1's reading of a K0 tape into K-form rows (its twin, the twins' `k0`);
//! - `residency`: the slabs' residency, `rerail_slabs`'s policy (the budget's least recently used, the drop after a
//!   run);
//! - `answer`: Δanswer's operations, `answer_load` (the file layer as it is) and `parse` (the rr parser), host versions;
//! - `boot`: B§8's run, its first two lines: the tape as a world read, then L1;
//! - `l1`: L1, the boot operation: a delta operator of one tape, no program, K-form as its production and the symbols
//!   as facts in.
//!
//! Files are not read here: a document is answered through the engine's file layer (`files`: the observer, its
//! placed read into the placer's home, the rr parser's tape written into a tape block, a blob named by its content),
//! as *0686: Operations* §9 sets out. `text` makes a document of a source given in memory (tests).
//!
//! The milestones toward one boot operation, L1 (the rebuild plan's §4), come after: the operator contract (B3), l0's
//! tape through the file layer (B4), symbols (B5) and L1 itself (B6).

pub mod answer;
pub mod body;
pub mod boot;
pub mod cache;
pub mod contract;
pub mod dir;
pub mod drop;
pub mod files;
pub mod funcs;
pub mod intern;
pub mod k0;
pub mod l1;
pub mod kform;
pub mod op;
pub mod phase;
pub mod plan;
pub mod program;
pub mod residency;
pub mod shapes;
pub mod sym;
pub mod tally;
pub mod tape;
pub mod trace;

#[cfg(test)]
mod tests;

/// `bootstrap-l0.rerail`: the reader of rules (P2) and the rule that orders the bootstrap's modules.
pub const L0: &str = include_str!("../rerail/bootstrap-l0.rerail");
