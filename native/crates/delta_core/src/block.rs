//! An operation block's reference table and its body's start (moved from `rerail_blocks::gens`).
//!
//! The table lies right after the engine's 64-byte header: its mark, its count, then up to `MAX_REFS` references. The
//! body starts at `BODY_AT`; its first word is the bytes of the body its writer used.

use crate::pod::{Pod, word, words};

/// Where a block's reference table lies: right after the engine's 64-byte header.
pub const REFS_AT: usize = 64;
/// The most blocks one block refers to directly (a layer refers to the one below it; a merged layer to the one below
/// the merged ones).
pub const MAX_REFS: usize = 30;
/// Where an operation block's body starts: after the reference table (its mark and count, then `MAX_REFS` references),
/// on a cache line.
pub const BODY_AT: usize = 320;
const _: () = assert!(REFS_AT + 8 + MAX_REFS * size_of::<Ref>() <= BODY_AT);
/// The reference table's mark: a block made without one (the engine's other blocks) refers to nothing.
pub const REFS_MAGIC: u32 = u32::from_le_bytes(*b"REF0");

/// A block id: the directory's handle of a block, the same on every device; each device's table gives its address.
pub type Id = u32;

/// A reference: a block's id and the offset in it of what is referred to (32 bits: a block's offsets fit).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ref {
    pub id: Id,
    pub at: u32,
}

// SAFETY: two u32s, no padding.
unsafe impl Pod for Ref {}

/// The reference table of a block, read in place: what it refers to, each a block id and an offset in it. A block
/// without one refers to nothing.
pub fn refs(bytes: &[u8]) -> &[Ref] {
    if bytes.len() < BODY_AT || word::<u32>(bytes, REFS_AT) != REFS_MAGIC {
        return &[];
    }
    let n = word::<u32>(bytes, REFS_AT + 4) as usize;
    words::<Ref>(bytes, REFS_AT + 8, n.min(MAX_REFS))
}
