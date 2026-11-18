//! An atoms block (decision 0401): a run of consecutive atoms, the strings every id names, with everything a reader
//! needs nested inside: each atom's end in the strings, its hash, and an open-addressing table from hash to id. A
//! store keeps its atoms as a chain of these in files beside it, mapped by every process that opens it: a lookup
//! probes the table and compares the string where it lies, and no stored atom is ever copied onto a heap.
//!
//! One process appends (the store's atoms lease holder, `AtomsW`); any number read (`AtomsV`), each up to an end it
//! is told: the end the store's last commit reached. A slot or string past that end may be one being written, or a
//! crashed writer's, and is never read. The regions are fixed when the block is made (its file is sparse, so room
//! not yet written takes no disk): `ends`, `hashes` and `table` first, `strings` last, so that a frozen block gives
//! back the strings' unwritten tail by truncation.
//!
//! The order of an append makes a reader that sees a slot see its atom: the string, its end and its hash are written,
//! then the slot (a release store), then the count. A reader loads a slot with acquire.

use crate::block::{Block, FROZEN, PAGE, kind};
use crate::refs::Span;
use crate::structure;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering::*, fence};

structure! {
    /// An atoms block's root.
    pub struct AtomsRoot {
        /// The first id it holds.
        pub first: u64,
        /// How many atoms, and string bytes, it has room for.
        pub cap: u64,
        pub bytes_cap: u64,
        /// How many atoms it holds, and their bytes: written by its one writer as it appends. A reader trusts only
        /// the end it is told.
        pub count: AtomicU64,
        pub bytes_len: AtomicU64,
        /// The table's slots less one (a power of two less one).
        pub mask: u64,
        /// Each atom's end in `strings`.
        pub ends: Span<u64>,
        /// Each atom's string's hash, as the store computes it.
        pub hashes: Span<u64>,
        /// The slots: a hash's high half, then the id plus one (`0` is empty).
        pub table: Span<AtomicU64>,
        pub strings: Span<u8>,
    }
}

/// What reads a run of atoms, wherever it lies: one block, or a store's chain of them.
pub trait AtomRead {
    /// The ids it holds: from its first to the end it reads up to.
    fn ids(&self) -> std::ops::Range<u32>;
    fn str(&self, id: u32) -> Option<&str>;
    fn hash(&self, id: u32) -> Option<u64>;
    /// The id of `s`, whose hash is `h`, if it holds one below `ids().end`.
    fn find(&self, s: &str, h: u64) -> Option<u32>;
}

/// A block's regions as addresses, resolved once from its root.
#[derive(Clone, Copy)]
struct Raw {
    first: u32,
    cap: u32,
    bytes_cap: u64,
    mask: u64,
    ends: *const u64,
    hashes: *const u64,
    table: *const AtomicU64,
    strings: *const u8,
}

// SAFETY: the addresses are of a mapping its owner keeps alive; reads are of committed atoms only, and slots are
// atomics.
unsafe impl Send for Raw {}
unsafe impl Sync for Raw {}

/// A slot's value: the hash's high half, then the id plus one.
fn slot(h: u64, id: u32) -> u64 {
    (h & 0xFFFF_FFFF_0000_0000) | u64::from(id + 1)
}

impl Raw {
    fn of(b: &Block) -> Raw {
        let r: &AtomsRoot = b.root();
        Raw {
            first: r.first as u32,
            cap: r.cap as u32,
            bytes_cap: r.bytes_cap,
            mask: r.mask,
            ends: r.ends.as_ptr(),
            hashes: r.hashes.as_ptr(),
            table: r.table.as_ptr(),
            strings: r.strings.as_ptr(),
        }
    }

    /// The `i`th atom's string (`i` below the end read up to).
    fn str_at(&self, i: usize) -> &str {
        // SAFETY: `i` is below the count the caller read; each string was written whole, from a `&str`, before the
        // count covered it.
        unsafe {
            let start = if i == 0 { 0 } else { *self.ends.add(i - 1) as usize };
            let end = *self.ends.add(i) as usize;
            std::str::from_utf8_unchecked(std::slice::from_raw_parts(self.strings.add(start), end - start))
        }
    }

    fn str(&self, id: u32, end: u32) -> Option<&str> {
        (id >= self.first && id < end).then(|| self.str_at((id - self.first) as usize))
    }

    fn hash(&self, id: u32, end: u32) -> Option<u64> {
        // SAFETY: below the end read up to.
        (id >= self.first && id < end).then(|| unsafe { *self.hashes.add((id - self.first) as usize) })
    }

    fn find(&self, s: &str, h: u64, end: u32) -> Option<u32> {
        let tag = h & 0xFFFF_FFFF_0000_0000;
        let mut i = h & self.mask;
        loop {
            // SAFETY: `i` is masked to the table's slots.
            let v = unsafe { (*self.table.add(i as usize)).load(Acquire) };
            if v == 0 {
                return None;
            }
            if v & 0xFFFF_FFFF_0000_0000 == tag {
                let id = (v as u32).wrapping_sub(1);
                if id >= self.first && id < end && self.str_at((id - self.first) as usize) == s {
                    return Some(id);
                }
            }
            i = (i + 1) & self.mask;
        }
    }
}

/// The slots a block of `cap` atoms has: at least twice its atoms, a power of two.
fn slots(cap: u64) -> u64 {
    (cap * 2).next_power_of_two().max(16)
}

/// An open atoms block in a file: its one writer (the store's atoms lease holder).
pub struct AtomsW {
    block: Block,
    raw: Raw,
    /// Where it writes back without a shared mapping (WASI): the block lives in memory there.
    #[cfg(not(unix))]
    file: std::fs::File,
}

// SAFETY: one writer appends, through `&mut self`; reads through `&self` are of what it wrote.
unsafe impl Send for AtomsW {}
unsafe impl Sync for AtomsW {}

impl AtomsW {
    /// A new block at `path` for ids from `first`, with room for `cap` atoms and `bytes_cap` bytes of strings.
    pub fn create(path: &Path, first: u32, cap: u32, bytes_cap: u64) -> io::Result<AtomsW> {
        let cap64 = u64::from(cap.max(1));
        let n = slots(cap64);
        let need = 4 * PAGE + (cap64 * 16 + n * 8 + bytes_cap) as usize;
        #[cfg(unix)]
        let block = Block::file(path, kind::ATOMS, need)?;
        #[cfg(not(unix))]
        let block = Block::anon(kind::ATOMS, need);
        let root: *mut AtomsRoot = block.alloc_t(1);
        let ends: *mut u64 = block.alloc_t(cap64 as usize);
        let hashes: *mut u64 = block.alloc_t(cap64 as usize);
        let table: *mut AtomicU64 = block.alloc_t(n as usize);
        // The strings last, page-aligned: a frozen block truncates their unwritten tail.
        let strings: *mut u8 = block.ptr(block.alloc(bytes_cap as usize, PAGE));
        // SAFETY: every pointer is into this block, which is open and has no other writer.
        unsafe {
            let r = &mut *root;
            r.first = u64::from(first);
            r.cap = cap64;
            r.bytes_cap = bytes_cap;
            r.mask = n - 1;
            Span::set(&r.ends, ends, cap64 as usize);
            Span::set(&r.hashes, hashes, cap64 as usize);
            Span::set(&r.table, table, n as usize);
            Span::set(&r.strings, strings, bytes_cap as usize);
        }
        block.set_root(root);
        let raw = Raw::of(&block);
        let w = AtomsW {
            block,
            raw,
            #[cfg(not(unix))]
            file: std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(true).open(path)?,
        };
        w.sync()?;
        Ok(w)
    }

    /// The open block at `path`, for its next writer, which continues from `end` (the end the store's last commit
    /// reached): what a writer appended past it (one that crashed before its commit) is taken back, slots and all.
    pub fn reopen(path: &Path, end: u32) -> io::Result<AtomsW> {
        let block = Block::map_file(path, kind::ATOMS, true)?;
        if block.header().state == FROZEN {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("{}: frozen, not appended to", path.display())));
        }
        let raw = Raw::of(&block);
        let r: &AtomsRoot = block.root();
        let held = r.count.load(Acquire);
        if u64::from(end) < u64::from(raw.first) || u64::from(end) > u64::from(raw.first) + held {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("{}: the store's end {end} is not within its atoms {}..{}", path.display(), raw.first, u64::from(raw.first) + held)));
        }
        let keep = u64::from(end - raw.first);
        if keep < held {
            for i in 0..=raw.mask {
                // SAFETY: `i` is within the slots; this writer is the block's only one.
                let s = unsafe { &*raw.table.add(i as usize) };
                let v = s.load(Relaxed);
                if v != 0 && (v as u32).wrapping_sub(1) >= end {
                    s.store(0, Relaxed);
                }
            }
            // A slot cleared from a probe run would end it early for an atom placed past it: those after are placed
            // again.
            Self::replace_runs(&raw);
            // SAFETY: below the count held.
            let bytes = if keep == 0 { 0 } else { unsafe { *raw.ends.add(keep as usize - 1) } };
            r.bytes_len.store(bytes, Release);
            r.count.store(keep, Release);
        }
        Ok(AtomsW {
            block,
            raw,
            #[cfg(not(unix))]
            file: std::fs::OpenOptions::new().read(true).write(true).open(path)?,
        })
    }

    /// Places again every slot of the table after an empty one that its probe run passed (`reopen`'s clearing): each
    /// slot taken out and inserted from its home.
    fn replace_runs(raw: &Raw) {
        let n = raw.mask + 1;
        let taken: Vec<u64> = (0..n)
            .filter_map(|i| {
                // SAFETY: within the slots; one writer.
                let s = unsafe { &*raw.table.add(i as usize) };
                let v = s.swap(0, Relaxed);
                (v != 0).then_some(v)
            })
            .collect();
        for v in taken {
            let id = (v as u32).wrapping_sub(1);
            // SAFETY: a slot's id is of an atom the block holds.
            let h = unsafe { *raw.hashes.add((id - raw.first) as usize) };
            Self::place(raw, h, v);
        }
    }

    fn place(raw: &Raw, h: u64, v: u64) {
        let mut i = h & raw.mask;
        loop {
            // SAFETY: masked to the slots.
            let s = unsafe { &*raw.table.add(i as usize) };
            if s.load(Relaxed) == 0 {
                s.store(v, Release);
                return;
            }
            i = (i + 1) & raw.mask;
        }
    }

    /// Appends `s` (its hash `h`): its id, or `None` when the block is full (the caller starts the next).
    pub fn append(&mut self, s: &str, h: u64) -> Option<u32> {
        let r: &AtomsRoot = self.block.root();
        let n = r.count.load(Relaxed);
        let at = r.bytes_len.load(Relaxed);
        if n >= u64::from(self.raw.cap) || at + s.len() as u64 > self.raw.bytes_cap {
            return None;
        }
        let id = self.raw.first + n as u32;
        let end = at + s.len() as u64;
        // SAFETY: within the regions' room, checked above; this writer is the block's only one, and no reader reads
        // past the end it is told, which this atom is not below yet.
        unsafe {
            std::ptr::copy_nonoverlapping(s.as_ptr(), self.raw.strings.add(at as usize) as *mut u8, s.len());
            *(self.raw.ends.add(n as usize) as *mut u64) = end;
            *(self.raw.hashes.add(n as usize) as *mut u64) = h;
        }
        fence(Release);
        Self::place(&self.raw, h, slot(h, id));
        r.bytes_len.store(end, Release);
        r.count.store(n + 1, Release);
        Some(id)
    }

    /// One past the last id appended.
    pub fn end(&self) -> u32 {
        self.raw.first + self.block.root::<AtomsRoot>().count.load(Acquire) as u32
    }

    /// Every append so far on the disk, before the commit that names them: the dirty pages written back, then the
    /// file flushed (a barrier on macOS, `fdatasync` elsewhere). Without shared mappings, the block's bytes are
    /// written to its file.
    pub fn sync(&self) -> io::Result<()> {
        #[cfg(unix)]
        {
            // SAFETY: the block's own mapping, its whole length.
            if unsafe { libc::msync(self.block.base_ptr().cast(), self.block.reserved(), libc::MS_SYNC) } != 0 {
                return Err(io::Error::last_os_error());
            }
            let Some(f) = self.block.mapped_file() else { return Ok(()) };
            use std::os::fd::AsRawFd as _;
            #[cfg(target_os = "macos")]
            let done = unsafe { libc::fcntl(f.as_raw_fd(), libc::F_BARRIERFSYNC) };
            #[cfg(not(target_os = "macos"))]
            let done = unsafe { libc::fdatasync(f.as_raw_fd()) };
            if done != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
        #[cfg(not(unix))]
        {
            use std::io::{Seek, Write};
            let mut f = &self.file;
            f.seek(io::SeekFrom::Start(0))?;
            f.write_all(self.block.bytes(0, self.block.used()))?;
            f.sync_data()
        }
    }

    /// Freezes the block: no more appends. Its strings' unwritten tail is given back (the file truncated), and its
    /// header says frozen, with its length.
    pub fn freeze(self) -> io::Result<()> {
        let r: &AtomsRoot = self.block.root();
        let strings_at = self.block.offset_of(self.raw.strings);
        let len = (strings_at as u64 + r.bytes_len.load(Acquire)).div_ceil(PAGE as u64) * PAGE as u64;
        // SAFETY: the block is open and this is its one writer.
        unsafe {
            let h = self.block.header_mut();
            h.length = len;
            fence(Release);
            std::ptr::write_volatile(&mut h.state, FROZEN);
        }
        self.sync()?;
        #[cfg(unix)]
        if let Some(f) = self.block.mapped_file() {
            f.set_len(len)?;
        }
        #[cfg(not(unix))]
        self.file.set_len(len)?;
        Ok(())
    }
}

impl AtomRead for AtomsW {
    fn ids(&self) -> std::ops::Range<u32> {
        self.raw.first..self.end()
    }
    fn str(&self, id: u32) -> Option<&str> {
        self.raw.str(id, self.end())
    }
    fn hash(&self, id: u32) -> Option<u64> {
        self.raw.hash(id, self.end())
    }
    fn find(&self, s: &str, h: u64) -> Option<u32> {
        self.raw.find(s, h, self.end())
    }
}

/// An atoms block mapped read-only, read up to an end it is told (the store's committed end within it).
pub struct AtomsV {
    _block: Block,
    raw: Raw,
    end: u32,
}

// SAFETY: read-only; reads are of atoms below `end`, written before it was told.
unsafe impl Send for AtomsV {}
unsafe impl Sync for AtomsV {}

impl AtomsV {
    /// The block at `path`, read up to `end` (at most what it holds).
    pub fn open(path: &Path, end: u32) -> io::Result<AtomsV> {
        let block = Block::map_file(path, kind::ATOMS, false)?;
        let raw = Raw::of(&block);
        let mut v = AtomsV { _block: block, raw, end: raw.first };
        v.extend(end)?;
        Ok(v)
    }

    /// Reads up to `end` now: a later commit of the block's writer.
    pub fn extend(&mut self, end: u32) -> io::Result<()> {
        if end < self.raw.first || end - self.raw.first > self.raw.cap {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("an end of {end} for atoms from {} (room for {})", self.raw.first, self.raw.cap)));
        }
        // What the writer wrote before the commit that named `end` is visible once its count is.
        let _ = self._block.root::<AtomsRoot>().count.load(Acquire);
        self.end = self.end.max(end);
        Ok(())
    }

    /// Its first id.
    pub fn first(&self) -> u32 {
        self.raw.first
    }

    /// Mapped bytes, for the profile (none of them heap).
    pub fn mapped(&self) -> usize {
        self._block.reserved()
    }
}

impl AtomRead for AtomsV {
    fn ids(&self) -> std::ops::Range<u32> {
        self.raw.first..self.end
    }
    fn str(&self, id: u32) -> Option<&str> {
        self.raw.str(id, self.end)
    }
    fn hash(&self, id: u32) -> Option<u64> {
        self.raw.hash(id, self.end)
    }
    fn find(&self, s: &str, h: u64) -> Option<u32> {
        self.raw.find(s, h, self.end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(s: &str) -> u64 {
        use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};
        BuildHasherDefault::<DefaultHasher>::default().hash_one(s)
    }

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("rerail-atoms-block-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A-R1: appended strings are found, read and hashed; one not appended is not found; a full block refuses.
    #[test]
    fn an_atoms_block_finds_what_it_holds() {
        let d = dir("finds");
        let mut w = AtomsW::create(&d.join("d0.blk"), 10, 1000, 1 << 16).unwrap();
        let words: Vec<String> = (0..1000).map(|i| format!("word-{i}")).chain(["".to_owned()]).collect();
        for (i, s) in words[..1000].iter().enumerate() {
            assert_eq!(w.append(s, h(s)), Some(10 + i as u32));
        }
        assert_eq!(w.append("one more", h("one more")), None, "full at its cap");
        assert_eq!(w.ids(), 10..1010);
        for (i, s) in words[..1000].iter().enumerate() {
            let id = 10 + i as u32;
            assert_eq!(w.find(s, h(s)), Some(id), "{s}");
            assert_eq!(w.str(id), Some(s.as_str()));
            assert_eq!(w.hash(id), Some(h(s)));
        }
        assert_eq!(w.find("absent", h("absent")), None);
        assert_eq!(w.str(9), None);
        assert_eq!(w.str(1010), None);
        // Room for bytes runs out too.
        let mut small = AtomsW::create(&d.join("d1.blk"), 0, 100, 8).unwrap();
        assert_eq!(small.append("12345678", h("12345678")), Some(0));
        assert_eq!(small.append("9", h("9")), None, "full at its bytes");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A-R2: a view reads and finds only below the end it is told; after `extend` it reads further. A writer that
    /// reopens at a committed end takes back what was appended past it.
    #[test]
    fn a_reader_reads_only_the_committed_end() {
        let d = dir("end");
        let path = d.join("d0.blk");
        let mut w = AtomsW::create(&path, 0, 64, 4096).unwrap();
        for s in ["a", "b", "c"] {
            w.append(s, h(s));
        }
        w.sync().unwrap();
        let mut v = AtomsV::open(&path, 2).unwrap();
        assert_eq!(v.find("b", h("b")), Some(1));
        assert_eq!(v.find("c", h("c")), None, "past the end it was told");
        assert_eq!(v.str(2), None);
        v.extend(3).unwrap();
        assert_eq!(v.find("c", h("c")), Some(2));
        assert_eq!(v.str(2), Some("c"));
        // A writer that crashed after appending "d": the store's end is 3.
        w.append("d", h("d"));
        drop(w);
        let mut again = AtomsW::reopen(&path, 3).unwrap();
        assert_eq!(again.end(), 3);
        assert_eq!(again.find("d", h("d")), None, "an append past the committed end is taken back");
        assert_eq!(again.append("e", h("e")), Some(3), "its id is the next writer's");
        assert_eq!(again.find("e", h("e")), Some(3));
        for (s, id) in [("a", 0), ("b", 1), ("c", 2)] {
            assert_eq!(again.find(s, h(s)), Some(id));
        }
        // Frozen: the file shrinks to what it holds, and still reads.
        again.sync().unwrap();
        let before = std::fs::metadata(&path).unwrap().len();
        again.freeze().unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() < before);
        let frozen = AtomsV::open(&path, 4).unwrap();
        assert_eq!(frozen.find("e", h("e")), Some(3));
        assert_eq!(frozen.str(0), Some("a"));
        assert!(AtomsW::reopen(&path, 4).is_err(), "a frozen block takes no appends");
        let _ = std::fs::remove_dir_all(&d);
    }
}
