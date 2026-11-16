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
        let root: *mut JournalRoot = block.alloc_t(1);
        let locs: *mut EntryLoc = block.alloc_t(cap.max(1));
        unsafe { Span::set(&raw mut (*root).entries, locs, cap.max(1)) };
        block.set_root(root);
        Ok(Journal { block })
    }

    /// Copies an entry's range in from the scratch block it was built in. Safe from many threads at once.
    pub fn add(&self, src: &Block, start: usize, end: usize, entry: usize, key: u32, digest: u64) {
        let dst = self.block.copy_in(src.bytes(start, end), start);
        let root: &JournalRoot = self.block.root();
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

    pub fn freeze(self) -> Frozen {
        self.block.freeze()
    }
}

pub fn entries(b: &Block) -> &[EntryLoc] {
    let r: &JournalRoot = b.root();
    &r.entries.as_slice()[..r.n.load(Acquire) as usize]
}

pub fn open(path: &Path) -> Result<Frozen, String> {
    Block::open_file(path, kind::JOURNAL)
}
