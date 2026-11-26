//! Generations by reference, and the descriptor table a launch reads them through (decision 0686; *0686: Operations*
//! §2, E2).
//!
//! **References.** A block an operation uses refers to other blocks by **block id and offset** (O§2: "A block refers to
//! another by block id and offset, in its header"), in its reference table, which lies at a fixed place right after
//! the engine's header (`REFS_AT`): the header itself is the engine's, read by every block, and stays as it is. The
//! offset is where, in the block referred to, the structure referred to lies (a layer's header, at `BODY_AT`). A block refers only to frozen blocks, so the one-writer rule holds across references;
//! the host's directory (`rerail_delta_exec::dir`) checks that, keeps a `Frozen` handle for each reference, and read-
//! pins what a block refers to whenever the block is read.
//!
//! **The descriptor table.** For each launch the host writes the table of the device the operation runs on: per block
//! id the operation can reach (its slots, and what they refer to, transitively), the address of that block's version
//! in the device's domain, its length, kind and version. A kernel follows a reference through the table of its own
//! device. On the CPU the table holds host addresses, and `Table` is how a twin follows one: an id gives the bytes of
//! a block pinned for the launch, in place.
//!
//! A device's table is the same format, its rows' addresses the blocks' device addresses (`device_rows`), in a block
//! of its own bound to the kernel; a kernel finds a block's row by id through `DESC_READS`, `Table::row`'s device
//! half, and reads the block where it lies through the row's address.
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

/// The reference table of a block, read in place: what it refers to, each a block id and an offset in it. A block
/// without one refers to nothing.
pub fn refs(bytes: &[u8]) -> &[Ref] {
    if bytes.len() < BODY_AT || word::<u32>(bytes, REFS_AT) != REFS_MAGIC {
        return &[];
    }
    let n = word::<u32>(bytes, REFS_AT + 4) as usize;
    words::<Ref>(bytes, REFS_AT + 8, n.min(MAX_REFS))
}

/// Lays an empty reference table out in a new block's bytes.
pub fn init_refs(bytes: &mut [u8]) {
    set_word::<u32>(bytes, REFS_AT, REFS_MAGIC);
    set_word::<u32>(bytes, REFS_AT + 4, 0);
}

/// Adds a reference to what lies at `at` in block `id`: `Err` past `MAX_REFS`, or in a block without a table.
pub fn add_ref(bytes: &mut [u8], id: Id, at: usize) -> Result<(), String> {
    let at = u32::try_from(at).map_err(|_| format!("a reference to block #{id} at {at}, past 32 bits"))?;
    if word::<u32>(bytes, REFS_AT) != REFS_MAGIC {
        return Err("a block made without a reference table".into());
    }
    let n = word::<u32>(bytes, REFS_AT + 4) as usize;
    if n == MAX_REFS {
        return Err(format!("a block refers to at most {MAX_REFS} blocks"));
    }
    set_word(bytes, REFS_AT + 8 + size_of::<Ref>() * n, Ref { id, at });
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
    8 + size_of_val(rows)
}

/// Writes a descriptor table at the start of `bytes` (a table block's body): rows sorted by id.
pub fn write_table(bytes: &mut [u8], rows: &[Desc]) {
    set_word::<u64>(bytes, 0, rows.len() as u64);
    words_mut::<Desc>(bytes, 8, rows.len()).copy_from_slice(rows); // copy: derived (the launch's descriptors, written once into its table block)
    words_mut::<Desc>(bytes, 8, rows.len()).sort_by_key(|d| d.id);
}

/// A device's descriptor table rows for `rows` (a launch's CPU rows, over `pins`): each block's address in the device's
/// domain, its buffer the block's own (`Block::device_buffer`: its memory wrapped in place, nothing uploaded), and the
/// length that buffer covers. With the buffers, which a launch names as used through addresses (`uses_read`).
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
pub fn device_rows(rows: &[Desc], pins: &[&BlockPin], device: &rerail_device::device::Device) -> Result<(Vec<Desc>, Vec<rerail_device::device::Buffer>), String> {
    let mut out = Vec::with_capacity(rows.len());
    let mut buffers = Vec::with_capacity(rows.len());
    for d in rows {
        let pin = pins.iter().find(|p| p.addr() as u64 == d.addr).ok_or_else(|| format!("descriptor table row #{} names memory the launch did not pin", d.id))?;
        let b = pin.block();
        let buffer = b.device_buffer(device)?;
        let addr = buffer.device_address().ok_or_else(|| format!("block #{}: the device gives its buffers no address", d.id))?;
        let len = b.kept_device_buffer().map_or(d.len, |(_, n)| (n as u64).min(d.len));
        out.push(Desc { addr, len, ..*d });
        buffers.push(buffer);
    }
    Ok((out, buffers))
}

/// A launch's table rows in memory domain `domain` (a discrete GPU's, 1 and up: decision 0440 §7.2): each block's
/// address its replica's there (`Directory::replicate`, `BlockPin::copy_address`), its length the block's. Refused
/// when a block the launch reaches has no replica there, or one with no address.
pub fn device_rows_in(rows: &[Desc], pins: &[&BlockPin], domain: u32) -> Result<Vec<Desc>, String> {
    assert_ne!(domain, 0, "the host's memory: the rows as they are (`device_rows` wraps them in place)");
    rows.iter()
        .map(|d| {
            let pin = pins.iter().find(|p| p.addr() as u64 == d.addr).ok_or_else(|| format!("descriptor table row #{} names memory the launch did not pin", d.id))?;
            if !pin.has_copy_in(domain) {
                return Err(format!("block #{} has no replica in domain {domain}", d.id));
            }
            let addr = pin.copy_address(domain).ok_or_else(|| format!("block #{}: its replica in domain {domain} has no address", d.id))?;
            Ok(Desc { addr, ..*d })
        })
        .collect()
}

/// A launch's descriptor table on a device (E2: a kernel follows a reference through its own device's table): the same
/// rows as its CPU table, each block's address the device's (`device_rows`), in a block of its own that the kernel
/// binds, and the buffers the kernel reaches through it. It reads the launch's blocks, so it lives no longer than the
/// launch's pins: the caller keeps the launch until the device's work completes.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
pub struct DeviceTable<'l> {
    _pins: std::marker::PhantomData<&'l BlockPin>,
    pin: BlockPin,
    /// Where the table lies in its block.
    pub at: u64,
    /// The table block's buffer, to bind.
    pub buffer: rerail_device::device::Buffer,
    /// The blocks' buffers, which the kernel reads through their addresses (`uses_read`).
    pub reached: Vec<rerail_device::device::Buffer>,
}

#[cfg(any(target_os = "macos", target_os = "linux", windows))]
impl DeviceTable<'_> {
    /// Hands the table's block to the device (E1): read by `op` until `fence` completes, as the launch's slots are
    /// (`Launch::to_fence`, after this: the table reads the launch's pins).
    pub fn to_fence(self, fence: &std::sync::Arc<crate::ledger::Fence>, op: &'static str) -> Result<(), String> {
        self.pin.to_fence(fence, op, 0).map_err(|(_, e)| e)
    }

    /// Its rows, by id ascending, as the device reads them.
    pub fn rows(&self) -> &[Desc] {
        let b = self.pin.bytes();
        let n = word::<u64>(b, self.at as usize) as usize;
        words::<Desc>(b, self.at as usize + 8, n)
    }
}

/// A launch's table (`table`, over `pins`: every block it pins) on `device`.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
pub fn device_table<'l>(table: &Table<'l>, pins: &[&'l BlockPin], device: &rerail_device::device::Device) -> Result<DeviceTable<'l>, String> {
    use crate::block::{Block, PAGE, kind};
    let (rows, reached) = device_rows(table.rows(), pins, device)?;
    let block = std::sync::Arc::new(Block::anon(kind::DESC, BODY_AT + table_bytes(&rows) + PAGE));
    let at = block.alloc(table_bytes(&rows), 64);
    {
        let mut w = crate::region::pin_block(&block, "device descriptor table", true).expect("a new block is unused");
        write_table(&mut w.bytes_mut().expect("a write pin")[at..], &rows);
    }
    let pin = crate::region::pin_block(&block, "device descriptor table", false).expect("the table is the launch's");
    let buffer = block.device_buffer(device)?;
    Ok(DeviceTable { _pins: std::marker::PhantomData, pin, at: at as u64, buffer, reached })
}

/// A descriptor table's device reads, in UKL (decision 0440: every backend's): a row as `Desc` lies, its address a
/// pointer into the block's memory on the device, and the row of an id, found as `Table::row` finds it (the rows by
/// id ascending). After `<ukl/backend.h>`.
pub const DESC_READS: &str = r#"
#include <ukl/clif.h>
typedef struct DescRow { uint32_t id; uint32_t kind; uint64_t version; CLIF_BLOCK_SPACE const uint8_t* addr; uint64_t len; } DescRow;
UKL_STATIC_ASSERT(sizeof(DescRow) == 32, "DescRow: gens::Desc's size");
// Block `id`'s row in the table at `t` (its row count, then its rows), or none (0) when the launch cannot reach it.
UKL_INLINE CLIF_BLOCK_SPACE const DescRow* desc_row(CLIF_BLOCK_SPACE const uint8_t* t, uint32_t id) {
    uint64_t n = *(CLIF_BLOCK_SPACE const uint64_t*)t;
    CLIF_BLOCK_SPACE const DescRow* rows = (CLIF_BLOCK_SPACE const DescRow*)(t + 8);
    uint64_t lo = 0, hi = n;
    while (lo < hi) {
        uint64_t m = (lo + hi) / 2u;
        if (rows[m].id < id) lo = m + 1u; else hi = m;
    }
    return lo < n && rows[lo].id == id ? rows + lo : 0;
}
"#;

/// The CPU's descriptor table of one launch, as a twin follows it: an id gives the bytes of a block pinned for the
/// launch, where they lie. Made over the table's own pinned block and the pins of every block it names; each row is
/// checked to be one of those pins, so a row cannot name memory the launch has not pinned.
pub struct Table<'a> {
    rows: &'a [Desc],
    pins: &'a [BlockPin],
    written: &'a [u64],
}

impl<'a> Table<'a> {
    /// The table written at `at` in `table`'s bytes, over `pins` (read), and `written`, the addresses of the
    /// launch's slots pinned to write: their rows are in the table (O§2: "per block id it can reach (its slots, and
    /// what they refer to, transitively)"), and a host read of one through it finds none (its writer reads it through
    /// its own slot). Panics on a row neither holds.
    pub fn new(table: &'a BlockPin, at: usize, pins: &'a [BlockPin], written: &'a [u64]) -> Table<'a> {
        let b = table.bytes();
        let n = word::<u64>(b, at) as usize;
        let rows = words::<Desc>(b, at + 8, n);
        for d in rows {
            assert!(
                pins.iter().any(|p| p.addr() as u64 == d.addr && p.len() as u64 == d.len) || written.contains(&d.addr),
                "descriptor table row #{} names memory the launch did not pin",
                d.id
            );
        }
        Table { rows, pins, written }
    }

    /// Whether block `id` is a slot of the launch pinned to write.
    pub fn is_written(&self, id: Id) -> bool {
        self.row(id).is_some_and(|d| self.written.contains(&d.addr))
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
