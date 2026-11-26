//! An operator cache's header (P§11.3; the rebuild plan's B3: "blocks with P§11.3's header, kept by the cache_open and
//! cache_commit operations"). A cache is not a bistate: it is blocks mutated in place, one state, and its header names
//! the state it describes, the digest of the inputs it was made from. A run marks the header open before its first
//! write (`cache_open`); at commit it writes the new state's digest and clears the mark, last (`cache_commit`). A reader
//! that finds the mark open, or another state's digest, reads the cache as ∅. So a run that dies part-way costs the
//! next run the cache, and nothing else.
//!
//! The header's body: its length, the state's digest (0: ∅'s, a header just made), the open mark, and whether the
//! last `cache_open` found the cache holding the state it was asked for.

use crate::{Ctx, Tile, TwinPhase};
use rerail_blocks::gens::{BODY_AT, set_word, word};

crate::params! {
    /// `cache_open`'s state the run starts from; `cache_commit`'s state it made.
    pub struct CacheP {
        pub state: u64,
    }
}

/// The header's body bytes.
pub const CACHE_BYTES: usize = 32;
const STATE_AT: usize = BODY_AT + 8;
const OPEN_AT: usize = BODY_AT + 16;
const HELD_AT: usize = BODY_AT + 24;
/// The header: the cache operations' one slot.
const HEADER: usize = 0;

/// The state the header names.
pub fn state(header: &[u8]) -> u64 {
    word::<u64>(header, STATE_AT)
}

/// Whether a run marked the header open and has not committed.
pub fn is_open(header: &[u8]) -> bool {
    word::<u64>(header, OPEN_AT) != 0
}

/// Whether the cache holds `state`: not open, and naming it.
pub fn holds(header: &[u8], state: u64) -> bool {
    !is_open(header) && self::state(header) == state
}

/// After `cache_open`: whether the cache held the state the run starts from (else it reads as ∅).
pub fn held(header: &[u8]) -> bool {
    word::<u64>(header, HELD_AT) != 0
}

fn open(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let from = <CacheP as crate::Params>::read(c.params).state;
    let ([], [h]) = c.views([], [HEADER]);
    let was = holds(h, from);
    set_word::<u64>(h, BODY_AT, CACHE_BYTES as u64);
    set_word::<u64>(h, HELD_AT, u64::from(was));
    set_word::<u64>(h, OPEN_AT, 1);
}

fn commit(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let to = <CacheP as crate::Params>::read(c.params).state;
    let ([], [h]) = c.views([], [HEADER]);
    assert!(is_open(h), "a cache committed by the run that opened it");
    set_word::<u64>(h, STATE_AT, to);
    // The mark cleared last: a reader that sees it clear sees the state's digest.
    set_word::<u64>(h, OPEN_AT, 0);
}

/// `cache_open`: one item, the header marked open.
pub const CACHE_OPEN: &[TwinPhase] = &[TwinPhase { name: "open", items: |_| 1, run: open, again: None }];

/// `cache_commit`: one item, the state written, then the mark cleared.
pub const CACHE_COMMIT: &[TwinPhase] = &[TwinPhase { name: "commit", items: |_| 1, run: commit, again: None }];
