//! The descriptor table a launch reaches blocks through (*0686: Operations* §2, E2; moved from `rerail_blocks::gens`,
//! whose `Table` checks each row against the launch's pins): a row a block the operation can reach (its slots and
//! what they refer to, transitively), by id ascending, with its address in the device's domain, length, kind and
//! version. A reference is a block id and an offset, never a pointer; a read follows it through the table of the
//! device it runs on (`Reach`), so the same block is found at another address on another device.

use crate::block::Id;
use crate::pod::{Pod, word, words};

/// One row of a descriptor table: a block a launch can reach, where it lies in the device's domain.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Desc {
    pub id: Id,
    pub kind: u32,
    pub version: u64,
    pub addr: u64,
    pub len: u64,
}

// SAFETY: plain integers, every byte a field's.
unsafe impl Pod for Desc {}

/// A descriptor table's rows written at `at` of a block's bytes: its row count, then its rows, by id ascending.
pub fn rows_at(bytes: &[u8], at: usize) -> &[Desc] {
    let n = word::<u64>(bytes, at) as usize;
    words::<Desc>(bytes, at + 8, n)
}

/// Block `id`'s row in `rows` (by id ascending), by binary search: the kernels' `desc_row`.
pub fn row(rows: &[Desc], id: Id) -> Option<&Desc> {
    rows.binary_search_by_key(&id, |d| d.id).ok().map(|i| &rows[i])
}

/// How a read follows a reference: a block id gives that block's bytes, where they lie, for the launch it runs in
/// (`None`: the launch cannot reach it); they live as long as the table is held. The host's table
/// (`rerail_blocks::gens::Table`) answers from the blocks it pinned, each row checked against a pin; a device's table
/// from its rows' addresses. Shared by the tiles of a launch, so `Sync`.
pub trait Reach: Sync {
    fn block(&self, id: Id) -> Option<&[u8]>;
}
