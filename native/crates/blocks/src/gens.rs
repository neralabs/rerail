//! Generations by reference, and the descriptor table a launch reads them through (decision 0686; *0686: Operations*
//! §2, E2).
//!
//! **References.** A block an operation uses refers to other blocks by **block id**, in its reference table, which
//! lies at a fixed place right after the engine's header (`REFS_AT`): the header itself is the engine's, read by every
//! block, and stays as it is. A block refers only to frozen blocks, so the one-writer rule holds across references;
//! the host's directory (`rerail_delta_exec::dir`) checks that, keeps a `Frozen` handle for each reference, and read-
//! pins what a block refers to whenever the block is read.
//!
//! **The descriptor table.** For each launch the host writes the table of the device the operation runs on: per block
//! id the operation can reach (its slots, and what they refer to, transitively), the address of that block's version
//! in the device's domain, its length, kind and version. A kernel follows a reference through the table of its own
//! device. On the CPU the table holds host addresses, and `Table` is how a twin follows one: an id gives the bytes of
//! a block pinned for the launch, in place.
//!
//! **Typed views.** A block's columns are fixed-width integers read where they lie: `words` and `words_mut` give a
//! run of a `Pod` type at a byte offset, checked for bounds and alignment; `atomics` gives atomic words of a block
//! pinned to write, for claims and cells many writers fill at once (host threads or a GPU's).
//!
//! An operation block's body starts at `BODY_AT`. Its first word is the bytes of the body its writer used, so the
//! host trims the block to them when it freezes it (`body_len`), reading that one word and no row.

use crate::Pod;
use crate::region::BlockPin;

/// Where a block's reference table lies: right after the engine's 64-byte header.
pub const REFS_AT: usize = 64;
/// The most blocks one block refers to directly (a layer refers to the one below it; a merged layer to the one below
/// the merged ones).
pub const MAX_REFS: usize = 30;
/// Where an operation block's body starts: after the reference table, on a cache line.
pub const BODY_AT: usize = 192;
/// The reference table's mark: a block made without one (the engine's other blocks) refers to nothing.
pub const REFS_MAGIC: u32 = u32::from_le_bytes(*b"REF0");

/// A block id: the directory's handle of a block, the same on every device; each device's table gives its address.
pub type Id = u32;

/// A run of `n` `T`s at byte `at` of `bytes`, read in place. Panics, naming the run, past the bytes or misaligned.
#[inline]
pub fn words<T: Pod>(bytes: &[u8], at: usize, n: usize) -> &[T] {
    let len = n * size_of::<T>();
    assert!(at + len <= bytes.len(), "a run of {n} words at {at} past a block's {} bytes", bytes.len());
    let p = bytes[at..].as_ptr();
    assert!((p as usize).is_multiple_of(align_of::<T>()), "a run of words at {at}, not aligned to {}", align_of::<T>());
    // SAFETY: in bounds and aligned (checked); `T: Pod` has no invalid bit patterns and holds no address.
    unsafe { std::slice::from_raw_parts(p as *const T, n) }
}

/// `words`, to write.
#[inline]
pub fn words_mut<T: Pod>(bytes: &mut [u8], at: usize, n: usize) -> &mut [T] {
    let len = n * size_of::<T>();
    assert!(at + len <= bytes.len(), "a run of {n} words at {at} past a block's {} bytes", bytes.len());
    let p = bytes[at..].as_mut_ptr();
    assert!((p as usize).is_multiple_of(align_of::<T>()), "a run of words at {at}, not aligned to {}", align_of::<T>());
    // SAFETY: as `words`, and the borrow is exclusive.
    unsafe { std::slice::from_raw_parts_mut(p as *mut T, n) }
}

/// One `T` at byte `at`.
#[inline]
pub fn word<T: Pod + Copy>(bytes: &[u8], at: usize) -> T {
    words::<T>(bytes, at, 1)[0]
}

/// Writes one `T` at byte `at`.
#[inline]
pub fn set_word<T: Pod>(bytes: &mut [u8], at: usize, v: T) {
    words_mut::<T>(bytes, at, 1)[0] = v;
}

/// The reference table of a block, read in place: the ids it refers to. A block without one refers to nothing.
pub fn refs(bytes: &[u8]) -> &[Id] {
    if bytes.len() < BODY_AT || word::<u32>(bytes, REFS_AT) != REFS_MAGIC {
        return &[];
    }
    let n = word::<u32>(bytes, REFS_AT + 4) as usize;
    words::<u32>(bytes, REFS_AT + 8, n.min(MAX_REFS))
}

/// Lays an empty reference table out in a new block's bytes.
pub fn init_refs(bytes: &mut [u8]) {
    set_word::<u32>(bytes, REFS_AT, REFS_MAGIC);
    set_word::<u32>(bytes, REFS_AT + 4, 0);
}

/// Adds a reference to block `id`: `Err` past `MAX_REFS`, or in a block without a table.
pub fn add_ref(bytes: &mut [u8], id: Id) -> Result<(), String> {
    if word::<u32>(bytes, REFS_AT) != REFS_MAGIC {
        return Err("a block made without a reference table".into());
    }
    let n = word::<u32>(bytes, REFS_AT + 4) as usize;
    if n == MAX_REFS {
        return Err(format!("a block refers to at most {MAX_REFS} blocks"));
    }
    set_word::<u32>(bytes, REFS_AT + 8 + 4 * n, id);
    set_word::<u32>(bytes, REFS_AT + 4, n as u32 + 1);
    Ok(())
}

/// The bytes of a body its writer used (its first word), from `BODY_AT`: at least the word itself.
pub fn body_len(bytes: &[u8]) -> usize {
    (word::<u64>(bytes, BODY_AT) as usize).max(8)
}

/// An atomic word a block holds (`BlockPin::atomics`).
pub trait Atomic: Pod + Sync {}
impl Atomic for std::sync::atomic::AtomicU32 {}
impl Atomic for std::sync::atomic::AtomicU64 {}
impl Atomic for std::sync::atomic::AtomicI64 {}

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

unsafe impl Pod for Desc {}

/// A descriptor table's bytes: its row count, then its rows, by id ascending.
pub fn table_bytes(rows: &[Desc]) -> usize {
    8 + rows.len() * size_of::<Desc>()
}

/// Writes a descriptor table at the start of `bytes` (a table block's body): rows sorted by id.
pub fn write_table(bytes: &mut [u8], rows: &[Desc]) {
    set_word::<u64>(bytes, 0, rows.len() as u64);
    words_mut::<Desc>(bytes, 8, rows.len()).copy_from_slice(rows); // copy: derived (the launch's descriptors, written once into its table block)
    words_mut::<Desc>(bytes, 8, rows.len()).sort_by_key(|d| d.id);
}

/// The CPU's descriptor table of one launch, as a twin follows it: an id gives the bytes of a block pinned for the
/// launch, where they lie. Made over the table's own pinned block and the pins of every block it names; each row is
/// checked to be one of those pins, so a row cannot name memory the launch has not pinned.
pub struct Table<'a> {
    rows: &'a [Desc],
    pins: &'a [BlockPin],
}

impl<'a> Table<'a> {
    /// The table written at `at` in `table`'s bytes, over `pins`. Panics on a row no pin holds.
    pub fn new(table: &'a BlockPin, at: usize, pins: &'a [BlockPin]) -> Table<'a> {
        let b = table.bytes();
        let n = word::<u64>(b, at) as usize;
        let rows = words::<Desc>(b, at + 8, n);
        for d in rows {
            assert!(
                pins.iter().any(|p| p.addr() as u64 == d.addr && p.len() as u64 == d.len),
                "descriptor table row #{} names memory the launch did not pin",
                d.id
            );
        }
        Table { rows, pins }
    }

    /// Its rows, by id ascending.
    pub fn rows(&self) -> &'a [Desc] {
        self.rows
    }

    /// Block `id`'s row.
    pub fn row(&self, id: Id) -> Option<&'a Desc> {
        self.rows.binary_search_by_key(&id, |d| d.id).ok().map(|i| &self.rows[i])
    }

    /// Block `id`'s bytes, in place: `None` when the launch cannot reach it.
    pub fn block(&self, id: Id) -> Option<&'a [u8]> {
        let d = self.row(id)?;
        self.pins.iter().find(|p| p.addr() as u64 == d.addr).map(|p| p.bytes())
    }
}
