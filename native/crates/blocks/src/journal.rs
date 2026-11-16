//! A `Journal` block: a wave's writes, in a file (decision 0260 §8). Writers copy finished ranges in at once
//! (allocation is atomic), the block is frozen (truncated, header last), and the OS writes the pages back.
//! A journal that does not check when read back (not frozen, or short) is refused, and its wave is redone.

use crate::block::{Block, Frozen, kind};
use crate::refs::Span;
use crate::structure;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering::{AcqRel, Acquire}};

structure! {
    /// One memo entry's place: its document key and content digest, its range, and its offset.
    pub struct EntryLoc {
        pub key: u32,
        pub pad: u32,
        pub digest: u64,
        pub start: u64,
        pub end: u64,
        pub entry: u64,
    }
}

structure! {
    pub struct JournalRoot {
        pub n: AtomicU32,
        pub pad: u32,
        pub entries: Span<EntryLoc>,
    }
}

pub struct Journal {
    pub block: Block,
}

impl Journal {
    pub fn create(path: &Path, reserve: usize, cap: usize) -> std::io::Result<Journal> {
        let block = Block::file(path, kind::JOURNAL, reserve)?;
        init(&block, cap);
        Ok(Journal { block })
    }

    /// Copies an entry's range in from the scratch block it was built in. Safe from many threads at once.
    pub fn add(&self, src: &Block, start: usize, end: usize, entry: usize, key: u32, digest: u64) {
        add(&self.block, src, start, end, entry, key, digest);
    }

    pub fn freeze(self) -> Frozen {
        self.block.freeze()
    }
}

/// Makes `block` (a journal's kind, just made: a file, or a deferrable slab of the pool) a journal of room for
/// `cap` entries.
pub fn init(block: &Block, cap: usize) {
    let root: *mut JournalRoot = block.alloc_t(1);
    let locs: *mut EntryLoc = block.alloc_t(cap.max(1));
    unsafe { Span::set(&raw mut (*root).entries, locs, cap.max(1)) };
    block.set_root(root);
}

/// `Journal::add` on any journal block (`init`).
pub fn add(journal: &Block, src: &Block, start: usize, end: usize, entry: usize, key: u32, digest: u64) {
    let dst = journal.copy_in(src.bytes(start, end), start);
    let root: &JournalRoot = journal.root();
    let i = root.n.fetch_add(1, AcqRel) as usize;
    assert!(i < root.entries.len(), "journal entries past its capacity");
    // The slot was just claimed: nothing reads it before this one write.
    let loc = unsafe { root.entries.as_ptr().add(i) as *mut EntryLoc };
    unsafe {
        *loc = EntryLoc {
            key,
            pad: 0,
            digest,
            start: dst as u64,
            end: (dst + end - start) as u64,
            entry: (dst + entry - start) as u64,
        }
    };
}

/// Copies `bytes` (one entry, its root at its start) into a journal block (`init`) at `align`, and records its place
/// under `key` and `digest`. Returns its offset. Safe from many threads at once.
pub fn put(journal: &Block, bytes: &[u8], align: usize, key: u32, digest: u64) -> usize {
    let dst = journal.alloc(bytes.len(), align);
    // SAFETY: `dst..dst + len` was just allocated in the block, for this copy alone.
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), journal.base_ptr().add(dst), bytes.len()) };
    let root: &JournalRoot = journal.root();
    let i = root.n.fetch_add(1, AcqRel) as usize;
    assert!(i < root.entries.len(), "journal entries past its capacity");
    let loc = unsafe { root.entries.as_ptr().add(i) as *mut EntryLoc };
    let end = (dst + bytes.len()) as u64;
    unsafe { *loc = EntryLoc { key, pad: 0, digest, start: dst as u64, end, entry: dst as u64 } };
    dst
}

/// The bytes `init`, `cap` entries and `put`s of `lens` at `align` take, at most: a block's reservation for them.
pub fn reserve_for(cap: usize, lens: impl Iterator<Item = usize>, align: usize) -> usize {
    let head = size_of::<crate::block::Header>().next_multiple_of(8) + size_of::<JournalRoot>().next_multiple_of(8) + cap.max(1) * size_of::<EntryLoc>();
    head + lens.map(|l| l + align).sum::<usize>() + align
}

pub fn entries(b: &Block) -> &[EntryLoc] {
    let r: &JournalRoot = b.root();
    &r.entries.as_slice()[..r.n.load(Acquire) as usize]
}

pub fn open(path: &Path) -> Result<Frozen, String> {
    Block::open_file(path, kind::JOURNAL)
}
