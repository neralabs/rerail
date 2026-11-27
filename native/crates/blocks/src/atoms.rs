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

use crate::block::{Block, FROZEN, Flush, PAGE, kind};
use rerail_delta_core::flat::RelSpan;
use crate::structure;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering::*, fence};

structure! {
    /// An atoms block's root.
    pub struct AtomsRoot {
        /// The first id it holds.
        pub first: u64,
        /// Its maker's tag for it, which the store's row of it records too: a block not the row's (a store copied
        /// without its blocks, or a stale file at its path) is refused, not read.
        pub tag: u64,
        /// How many atoms, and string bytes, it has room for.
        pub cap: u64,
        pub bytes_cap: u64,
        /// How many atoms it holds, and their bytes: written by its one writer as it appends. A reader trusts only
        /// the end it is told.
        pub count: AtomicU64,
        pub bytes_len: AtomicU64,
        /// The table's slots less one (a power of two less one).
        pub mask: u64,
        /// Slots taken: each append's, and those a crashed writer's appends left (passed over by every reader, and
        /// never cleared, since a reader may be probing past them). An append refuses at three quarters.
        pub placed: AtomicU64,
        /// Each atom's end in `strings`.
        pub ends: RelSpan<u64>,
        /// Each atom's string's hash, as the store computes it.
        pub hashes: RelSpan<u64>,
        /// The slots: a hash's high half, then the id plus one (`0` is empty).
        pub table: RelSpan<AtomicU64>,
        pub strings: RelSpan<u8>,
    }
}

/// An atoms structure laid out in an open block another made (a directory's block, decision 0686 B5), its root set
/// as the block's: the block must reserve `atoms_bytes(cap, bytes_cap)` past what it holds. Its one writer is then the
/// operation that holds the block to write (`append_in`), and its readers read it where it lies (`find_in`, `name_in`).
pub fn lay_out_in(block: &Block, first: u32, cap: u32, bytes_cap: u64) {
    AtomsW::lay_out(block, first, 0, cap, bytes_cap);
}

/// The bytes an atoms structure of `cap` atoms and `bytes_cap` string bytes takes.
pub fn atoms_bytes(cap: u32, bytes_cap: u64) -> usize {
    AtomsW::need(cap, bytes_cap)
}

/// The atoms root of a block, from its bytes where they lie (a pinned slot's: the block's own mapping).
fn root_in(bytes: &[u8]) -> &AtomsRoot {
    let at = crate::gens::word::<u64>(bytes, std::mem::offset_of!(crate::block::Header, root)) as usize;
    assert!(at != 0 && at + size_of::<AtomsRoot>() <= bytes.len(), "a block holding atoms (`lay_out_in`)");
    // SAFETY: the root `lay_out_in` set, inside the block's bytes, which are its mapping: its spans' self-relative
    // offsets reach the block's regions.
    unsafe { &*(bytes.as_ptr().add(at) as *const AtomsRoot) }
}

/// The id of `s` (its hash `h`) in the atoms block `bytes`, if it has one.
pub fn find_in(bytes: &[u8], s: &str, h: u64) -> Option<u32> {
    let r = root_in(bytes);
    let raw = Raw::of_root(r);
    raw.find(s, h, raw.first + r.count.load(Acquire) as u32)
}

/// The id of the string whose hash is `h` and which `eq` accepts, in the atoms block `bytes` (`find_in` for a string
/// read as it streams, compared where the atom lies).
pub fn find_in_by(bytes: &[u8], h: u64, eq: impl Fn(&str) -> bool) -> Option<u32> {
    let r = root_in(bytes);
    let raw = Raw::of_root(r);
    raw.find_by(h, eq, raw.first + r.count.load(Acquire) as u32)
}

/// The string of `id` in the atoms block `bytes`.
pub fn name_in(bytes: &[u8], id: u32) -> Option<&str> {
    let r = root_in(bytes);
    let raw = Raw::of_root(r);
    let s: *const str = raw.str(id, raw.first + r.count.load(Acquire) as u32)?;
    // SAFETY: the string lies in the block `bytes` is, which outlives this borrow of it.
    Some(unsafe { &*s })
}

/// How many atoms the block `bytes` holds.
pub fn count_in(bytes: &[u8]) -> u32 {
    root_in(bytes).count.load(Acquire) as u32
}

/// Appends `s` (its hash `h`) to the atoms block `bytes`, held to write by its one writer: its id, or `None` when the
/// block is full. As `AtomsW::append`.
pub fn append_in(bytes: &mut [u8], s: &str, h: u64) -> Option<u32> {
    let r = root_in(bytes);
    AtomsW::append_raw(r, &Raw::of_root(r), s, h)
}

/// Appends a string of `len` bytes (its hash `h`), which `fill` writes where the atom lies, as UTF-8, to the atoms
/// block `bytes` (`append_in` for a string read as it streams): its id, or `None` when the block is full.
pub fn append_in_by(bytes: &mut [u8], len: usize, h: u64, fill: impl FnOnce(&mut [u8])) -> Option<u32> {
    let r = root_in(bytes);
    AtomsW::append_raw_by(r, &Raw::of_root(r), len, h, fill)
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
        Raw::of_root(b.root())
    }

    fn of_root(r: &AtomsRoot) -> Raw {
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
        self.find_by(h, |t| t == s, end)
    }

    fn find_by(&self, h: u64, eq: impl Fn(&str) -> bool, end: u32) -> Option<u32> {
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
                if id >= self.first && id < end && eq(self.str_at((id - self.first) as usize)) {
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

/// An open atoms block: its one writer. In a file, the store's atoms lease holder's; in anonymous memory, a process's
/// pending atoms (decision 0478 step 2a), never written to a file.
pub struct AtomsW {
    block: std::sync::Arc<Block>,
    raw: Raw,
    /// Where it writes back without a shared mapping (WASI): the block lives in memory there. `None` for a block in
    /// anonymous memory.
    #[cfg(not(any(unix, windows)))]
    file: Option<std::fs::File>,
    /// How its write-backs reach the disk (`sync`, decision 0481).
    flush: Flush,
}

// SAFETY: one writer appends, through `&mut self`; reads through `&self` are of what it wrote.
unsafe impl Send for AtomsW {}
unsafe impl Sync for AtomsW {}

impl AtomsW {
    /// A new block at `path` for ids from `first`, with room for `cap` atoms and `bytes_cap` bytes of strings.
    pub fn create(path: &Path, first: u32, tag: u64, cap: u32, bytes_cap: u64, flush: Flush) -> io::Result<AtomsW> {
        let need = Self::need(cap, bytes_cap);
        #[cfg(any(unix, windows))]
        let block = Block::file(path, kind::ATOMS, need)?;
        #[cfg(not(any(unix, windows)))]
        let block = Block::anon(kind::ATOMS, need);
        let w = AtomsW {
            raw: Self::lay_out(&block, first, tag, cap, bytes_cap),
            block: std::sync::Arc::new(block),
            #[cfg(not(any(unix, windows)))]
            file: Some(std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(true).open(path)?),
            flush,
        };
        w.sync()?;
        Ok(w)
    }

    /// A new block in anonymous memory for ids from `first`, with room for `cap` atoms and `bytes_cap` bytes of
    /// strings: a process's pending atoms (decision 0478 step 2a). Its pages are taken as atoms are appended.
    pub fn anon(first: u32, cap: u32, bytes_cap: u64) -> AtomsW {
        let block = Block::anon(kind::ATOMS, Self::need(cap, bytes_cap));
        AtomsW {
            raw: Self::lay_out(&block, first, 0, cap, bytes_cap),
            block: std::sync::Arc::new(block),
            #[cfg(not(any(unix, windows)))]
            file: None,
            // Never written to a file: nothing to wait for.
            flush: Flush::None,
        }
    }

    /// The bytes a block of `cap` atoms and `bytes_cap` string bytes reserves.
    fn need(cap: u32, bytes_cap: u64) -> usize {
        let cap64 = u64::from(cap.max(1));
        4 * PAGE + (cap64 * 16 + slots(cap64) * 8 + bytes_cap) as usize
    }

    /// `block`'s regions laid out for ids from `first`: its root, set as the block's.
    fn lay_out(block: &Block, first: u32, tag: u64, cap: u32, bytes_cap: u64) -> Raw {
        let cap64 = u64::from(cap.max(1));
        let n = slots(cap64);
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
            r.tag = tag;
            r.cap = cap64;
            r.bytes_cap = bytes_cap;
            r.mask = n - 1;
            block.span(&r.ends, ends, cap64 as usize);
            block.span(&r.hashes, hashes, cap64 as usize);
            block.span(&r.table, table, n as usize);
            block.span(&r.strings, strings, bytes_cap as usize);
        }
        block.set_root(root);
        Raw::of(block)
    }

    /// The open block at `path`, for its next writer, which continues from `end` (the end the store's last commit
    /// reached, `resume`).
    pub fn reopen(path: &Path, end: u32, flush: Flush) -> io::Result<AtomsW> {
        let block = Block::map_file(path, kind::ATOMS, true)?;
        if block.header().state == FROZEN {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("{}: frozen, not appended to", path.display())));
        }
        let raw = Raw::of(&block);
        let mut w = AtomsW {
            block: std::sync::Arc::new(block),
            raw,
            #[cfg(not(any(unix, windows)))]
            file: Some(std::fs::OpenOptions::new().read(true).write(true).open(path)?),
            flush,
        };
        w.resume(end)?;
        Ok(w)
    }

    /// Continues from `end`, the end the store's last commit reached in this block: what was appended past it (by a
    /// writer that crashed or rolled back before its commit) is taken back. Its slots stay, passed over by every
    /// reader until an atom of its id is appended again (a slot's string is compared, so a stale one never answers):
    /// clearing them would end a probe early for a reader that is probing past them now.
    pub fn resume(&mut self, end: u32) -> io::Result<()> {
        let r: &AtomsRoot = self.block.root();
        let held = r.count.load(Acquire);
        let first = u64::from(self.raw.first);
        if u64::from(end) < first || u64::from(end) > first + held {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("the store's end {end} is not within a block's atoms {}..{}", first, first + held)));
        }
        let keep = u64::from(end) - first;
        if keep < held {
            // SAFETY: below the count held.
            let bytes = if keep == 0 { 0 } else { unsafe { *self.raw.ends.add(keep as usize - 1) } };
            r.bytes_len.store(bytes, Release);
            r.count.store(keep, Release);
        }
        Ok(())
    }

    /// Its first id, and the room it was made with (atoms, string bytes).
    pub fn first(&self) -> u32 {
        self.raw.first
    }
    pub fn tag(&self) -> u64 {
        self.block.root::<AtomsRoot>().tag
    }
    /// Whether it was frozen since this writer opened it (by another holder of its file): no more appends.
    pub fn frozen(&self) -> bool {
        // SAFETY: a read of the header's state, which a freeze writes once.
        unsafe { std::ptr::read_volatile(&self.block.header().state) == FROZEN }
    }
    pub fn cap(&self) -> u32 {
        self.raw.cap
    }
    pub fn bytes_cap(&self) -> u64 {
        self.raw.bytes_cap
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
        let raw = self.raw;
        Self::append_raw(self.block.root(), &raw, s, h)
    }

    fn append_raw(r: &AtomsRoot, raw: &Raw, s: &str, h: u64) -> Option<u32> {
        // copy: derived (an interned string, written once into the atoms)
        Self::append_raw_by(r, raw, s.len(), h, |to| to.copy_from_slice(s.as_bytes()))
    }

    /// `append_raw` of a string `fill` writes, `len` bytes of UTF-8, where the atom lies.
    fn append_raw_by(r: &AtomsRoot, raw: &Raw, len: usize, h: u64, fill: impl FnOnce(&mut [u8])) -> Option<u32> {
        let n = r.count.load(Relaxed);
        let at = r.bytes_len.load(Relaxed);
        if n >= u64::from(raw.cap) || at + len as u64 > raw.bytes_cap || r.placed.load(Relaxed) >= (raw.mask + 1) / 4 * 3 {
            return None;
        }
        let id = raw.first + n as u32;
        let end = at + len as u64;
        // SAFETY: within the regions' room, checked above; this writer is the block's only one, and no reader reads
        // past the end it is told, which this atom is not below yet.
        unsafe {
            let to = std::slice::from_raw_parts_mut(raw.strings.add(at as usize) as *mut u8, len);
            fill(to);
            assert!(std::str::from_utf8(to).is_ok(), "an atom's bytes are UTF-8");
            *(raw.ends.add(n as usize) as *mut u64) = end;
            *(raw.hashes.add(n as usize) as *mut u64) = h;
        }
        fence(Release);
        r.placed.fetch_add(1, Relaxed);
        Self::place(raw, h, slot(h, id));
        r.bytes_len.store(end, Release);
        r.count.store(n + 1, Release);
        Some(id)
    }

    /// The bytes its block has written (its pages taken).
    pub fn used(&self) -> usize {
        self.block.used()
    }

    /// The string bytes of ids `from..to` (within its atoms appended).
    pub fn bytes_of(&self, from: u32, to: u32) -> u64 {
        let end = self.end();
        let (from, to) = (from.clamp(self.raw.first, end), to.clamp(self.raw.first, end));
        if from >= to {
            return 0;
        }
        let at = |id: u32| {
            let i = (id - self.raw.first) as usize;
            // SAFETY: `i` is below the count appended.
            if i == 0 { 0 } else { unsafe { *self.raw.ends.add(i - 1) } }
        };
        at(to) - at(from)
    }

    /// Its block's memory: its first byte and its reservation's end.
    pub fn region(&self) -> (usize, usize) {
        (self.block.base_ptr() as usize, self.block.base_ptr() as usize + self.block.reserved())
    }

    /// One past the last id appended.
    pub fn end(&self) -> u32 {
        self.raw.first + self.block.root::<AtomsRoot>().count.load(Acquire) as u32
    }

    /// Every append so far written back for the commit that names them, as its `Flush` says (decision 0481): with
    /// `Synced`, on the disk (the dirty pages written back, then the file flushed: a barrier on macOS, `fdatasync`
    /// elsewhere); otherwise the OS's to write. Without shared mappings, the block's bytes are written to its file.
    pub fn sync(&self) -> io::Result<()> {
        #[cfg(any(unix, windows))]
        return self.block.sync_mapped(self.flush);
        #[cfg(not(any(unix, windows)))]
        {
            use std::io::{Seek, Write};
            let Some(mut f) = self.file.as_ref() else { return Ok(()) };
            f.seek(io::SeekFrom::Start(0))?;
            let bytes = self.block.bytes(0, self.block.used());
            // The kernel reads and writes a reservation's pages without faulting them in (Windows: `crate::windows`).
            crate::mem::commit(bytes.as_ptr().cast_mut(), bytes.len())?;
            f.write_all(bytes)?;
            if self.flush.waits() { f.sync_data() } else { Ok(()) }
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
        // The file cut once this writer's view is gone: Windows cuts no file a view maps (`block::cut`).
        #[cfg(any(unix, windows))]
        if let Some(f) = self.block.mapped_file().map(std::fs::File::try_clone).transpose()? {
            drop(self);
            crate::block::cut(&f, len)?;
            // Frozen: read-only on disk too (`mapped`, decision 0574).
            crate::mapped::mark_frozen(&f)?;
        }
        #[cfg(not(any(unix, windows)))]
        if let Some(f) = &self.file {
            f.set_len(len)?;
        }
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
    _block: std::sync::Arc<Block>,
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
        let mut v = AtomsV { _block: std::sync::Arc::new(block), raw, end: raw.first };
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

    /// Reads up to `end` now, nearer than before too: the end a transaction that appended and rolled back went back
    /// to (a fork's own appends, read inside the transaction that made them, decision 0170).
    pub fn set_end(&mut self, end: u32) -> io::Result<()> {
        self.end = self.raw.first;
        self.extend(end)
    }

    /// Its first id, and its maker's tag (`AtomsRoot::tag`).
    pub fn first(&self) -> u32 {
        self.raw.first
    }
    pub fn tag(&self) -> u64 {
        self._block.root::<AtomsRoot>().tag
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

/// A run of atoms where a block holds it, as a directory lists it (decision 0478 step 2b): ids `first..end` are read
/// in it; its atom `id`'s string ends at `ends[id - from]`, and begins where the one before ends (`0` for `from`'s).
#[derive(Clone, Copy, Debug)]
pub struct Run {
    pub first: u32,
    pub end: u32,
    /// The block's first id, which its `ends` count from.
    pub from: u32,
    pub ends: *const u64,
    pub strings: *const u8,
    /// How many atoms and string bytes the block has room for (its spans' lengths).
    pub cap: u32,
    pub bytes_cap: u64,
    /// The block's table from hash to id (`AtomsRoot::table`) and its slots (a power of two): a find by a string's
    /// hash (`find_in`, the CLIF's `atoms_find`) probes it.
    pub table: *const u64,
    pub slots: u64,
}

impl Raw {
    fn run(&self, first: u32, end: u32) -> Run {
        Run { first, end, from: self.first, ends: self.ends, strings: self.strings, cap: self.cap, bytes_cap: self.bytes_cap, table: self.table.cast(), slots: self.mask + 1 }
    }
}

impl AtomsW {
    /// Its atoms from `first` (at least its own first) as a directory's run: those appended until now.
    pub fn run_from(&self, first: u32) -> Run {
        self.raw.run(first.max(self.raw.first), self.end())
    }
    /// Its block, shared (a command buffer that reads it keeps it).
    pub fn block(&self) -> &std::sync::Arc<Block> {
        &self.block
    }
}

impl AtomsV {
    /// Its atoms below `end` (at most what it is read up to) as a directory's run.
    pub fn run_upto(&self, end: u32) -> Run {
        self.raw.run(self.raw.first, end.min(self.end))
    }
    /// Its block, shared (a command buffer that reads it keeps it).
    pub fn block(&self) -> &std::sync::Arc<Block> {
        &self._block
    }
}

structure! {
    /// A directory's entry (decision 0478 step 2b): a run of atoms, `first..end`, its block's first id (`from`), and
    /// where the block holds the run's `ends` and `strings`: the block by its descriptor's stable id (`desc`,
    /// `Block::desc_id`), and byte offsets into it. No address: each device places the block where it will, and a
    /// launch passes the block's descriptor as one of its arguments (`AtomsArgs`).
    pub struct DirEntry {
        pub first: u32,
        pub end: u32,
        pub from: u32,
        pub desc: u32,
        pub ends: u32,
        pub strings: u32,
        /// The run's `ends` (u64s) and string bytes its block has room for.
        pub nends: u32,
        pub nbytes: u32,
        /// The block's table from hash to id, an offset in it: a slot is the hash's high half, then the id plus
        /// one (`0` empty); `slots` of them, a power of two.
        pub table: u32,
        pub slots: u32,
    }
}

structure! {
    /// A directory of runs of atoms (decision 0478 step 2b), sorted by first id and not overlapping: the store's chain
    /// below the process's pending atoms, then theirs. The atoms reads (`atoms_at`, `atoms_len`, in CLIF) find an id's
    /// run by a binary search, through a launch's arguments (`AtomsArgs`).
    pub struct AtomsDirRoot {
        pub n: u32,
        pub pad: u32,
        pub entries: RelSpan<DirEntry>,
    }
}

/// The most blocks a launch binds: Metal's argument table has 31 entries, one of them the params'. A directory and the
/// atoms blocks it names past it is a launch error.
pub const LAUNCH_BLOCKS: usize = 30;

structure! {
    /// A launch's arguments to the atoms reads, in one reader's space (the host's, or one device's): the blocks it
    /// passes, each its descriptor's address (`args`), and the table of which argument is which descriptor, by its
    /// stable id (`ids`, ascending; `args` in the same order). A read takes an entry's `desc`, finds its argument in
    /// `ids`, and reads the block at that argument's address. The directory's own block is argument `dir`, its root
    /// at `root` in it. An address is the argument's offset from this record: a read's addresses are in its base's
    /// space (the record's, `t`). Made for each launch and device, held by the launch: no block readers share holds it.
    pub struct AtomsArgs {
        pub n: u32,
        pub dir: u32,
        pub root: u32,
        pub pad: u32,
        pub ids: [u32; LAUNCH_BLOCKS],
        pub args: [i64; LAUNCH_BLOCKS],
    }
}

/// A directory of runs of atoms in a block of its own (decision 0478 step 2b), and the blocks its entries name. It
/// holds no address: a launch passes it and them as arguments (`launch_blocks`, `bind`).
pub struct AtomsDir {
    block: Block,
    blocks: Vec<std::sync::Arc<Block>>,
}

// SAFETY: written once when made, before any reader is given it; read-only after.
unsafe impl Send for AtomsDir {}
unsafe impl Sync for AtomsDir {}

/// A launch's arguments (`AtomsArgs`) in a block of its own, for one reader's space: the table sent with the blocks.
pub struct AtomsBound {
    block: Block,
}

// SAFETY: as `AtomsDir`'s.
unsafe impl Send for AtomsBound {}
unsafe impl Sync for AtomsBound {}

impl AtomsBound {
    /// The record's address on the host: what the host's reads take (`str_in`, the JIT's `atoms_at`, `atoms_len`).
    pub fn root(&self) -> *const u8 {
        std::ptr::from_ref(self.block.root::<AtomsArgs>()).cast()
    }

    /// Its block (a device binds its buffer at `root_at`).
    pub fn block(&self) -> &Block {
        &self.block
    }

    pub fn root_at(&self) -> usize {
        self.block.offset_of(self.root())
    }
}

impl AtomsDir {
    /// `runs` (sorted by first id, not overlapping), each with the block that holds it, as a directory.
    pub fn new(runs: &[(Run, std::sync::Arc<Block>)]) -> AtomsDir {
        assert!(runs.windows(2).all(|w| w[0].0.end <= w[1].0.first), "a directory's runs sorted and apart");
        let mut blocks: Vec<std::sync::Arc<Block>> = Vec::new();
        let block = Block::anon(kind::ATOMS, 4 * PAGE + std::mem::size_of_val(runs) * 2 + 256);
        let root: *mut AtomsDirRoot = block.alloc_t(1);
        let entries: *mut DirEntry = block.alloc_t(runs.len().max(1));
        // SAFETY: the pointers are into this block, which no reader has yet.
        unsafe {
            for (k, (run, home)) in runs.iter().enumerate() {
                if !blocks.iter().any(|b| std::sync::Arc::ptr_eq(b, home)) {
                    blocks.push(std::sync::Arc::clone(home));
                }
                let off = |p: *const u8| {
                    assert!(home.holds(p), "a run's arrays in its block");
                    u32::try_from(home.offset_of(p)).expect("an atoms block's arrays within 4 GiB of its start")
                };
                entries.add(k).write(DirEntry {
                    first: run.first,
                    end: run.end,
                    from: run.from,
                    desc: home.desc_id(),
                    ends: off(run.ends.cast()),
                    strings: off(run.strings),
                    nends: run.cap,
                    nbytes: u32::try_from(run.bytes_cap).expect("an atoms block's strings within 4 GiB"),
                    table: off(run.table.cast()),
                    slots: u32::try_from(run.slots).expect("an atoms block's table within 2^32 slots"),
                });
            }
            (*root).n = runs.len() as u32;
            block.span(&raw const (*root).entries, entries, runs.len());
        }
        block.set_root(root);
        AtomsDir { block, blocks }
    }

    /// Its block, and the blocks its entries name.
    pub fn block(&self) -> &Block {
        &self.block
    }

    pub fn blocks(&self) -> &[std::sync::Arc<Block>] {
        &self.blocks
    }

    /// The root's offset in its block.
    pub fn root_at(&self) -> usize {
        self.block.offset_of(std::ptr::from_ref(self.block.root::<AtomsDirRoot>()))
    }

    /// The blocks a launch passes as its arguments, in order: its own, then each its entries name. More than
    /// `LAUNCH_BLOCKS` is a launch error, which names the count.
    pub fn launch_blocks(&self) -> Result<Vec<&Block>, String> {
        let n = self.blocks.len() + 1;
        if n > LAUNCH_BLOCKS {
            return Err(format!("an atoms directory over {n} blocks (its own and {} of atoms): a launch binds at most {LAUNCH_BLOCKS}", n - 1));
        }
        Ok(std::iter::once(&self.block).chain(self.blocks.iter().map(|b| &**b)).collect())
    }

    /// The launch's arguments for one reader's space: `descs[i]`, the address there of argument `i`'s descriptor
    /// (`launch_blocks`' order), passed with the table of which argument is which stable id, in a record of its own
    /// whose address there is `at(record's block)`.
    pub fn bind(&self, descs: &[u64], at: &dyn Fn(&Block) -> Option<u64>) -> Result<AtomsBound, String> {
        let blocks = self.launch_blocks()?;
        assert_eq!(descs.len(), blocks.len(), "a descriptor for each of the launch's blocks");
        let block = Block::anon(kind::ATOMS, 2 * PAGE);
        let t: *mut AtomsArgs = block.alloc_t(1);
        let here = at(&block).ok_or("the launch's arguments have no address in its space")? + block.offset_of(t) as u64;
        let mut order: Vec<usize> = (0..blocks.len()).collect();
        order.sort_by_key(|&i| blocks[i].desc_id());
        let mut args = AtomsArgs { n: blocks.len() as u32, dir: 0, root: self.root_at() as u32, pad: 0, ids: [0; LAUNCH_BLOCKS], args: [0; LAUNCH_BLOCKS] };
        for (k, &i) in order.iter().enumerate() {
            args.ids[k] = blocks[i].desc_id();
            args.args[k] = descs[i].wrapping_sub(here) as i64;
            if i == 0 {
                args.dir = k as u32;
            }
        }
        // SAFETY: the record's room, just allocated in its block, which no reader has yet.
        unsafe { t.write(args) };
        block.set_root(t);
        Ok(AtomsBound { block })
    }

    /// Its arguments on the host: each block's own memory.
    pub fn host_args(&self) -> Result<AtomsBound, String> {
        let descs: Vec<u64> = self.launch_blocks()?.iter().map(|b| b.base_ptr() as u64).collect();
        self.bind(&descs, &|b| Some(b.base_ptr() as u64))
    }
}

/// The string of `id` through the launch's arguments at `args` (host addresses: `AtomsDir::host_args`), as the
/// CLIF's `atoms_len` and `atoms_at` read it (the kernels' and the JIT's): the last run whose first id is at most `id`,
/// if `id` is below its end; its string ends at `ends[id - from]` and begins where the one before ends (`0` for the
/// run's first), both in the run's block: the argument whose stable id is the entry's. Id 0 is the empty string;
/// `None` for an id none of its runs holds.
///
/// # Safety
/// `args` is a live host record of a live directory, whose blocks are mapped.
pub unsafe fn str_in<'a>(args: *const u8, id: u32) -> Option<&'a [u8]> {
    if id == 0 {
        return Some(&[]);
    }
    // SAFETY: the caller's: a live record, its arguments in this address space, the directory's blocks mapped.
    unsafe {
        let t = &*args.cast::<AtomsArgs>();
        let arg = |desc: u32| {
            let ids = &t.ids[..t.n as usize];
            let k = ids.partition_point(|&d| d < desc);
            (ids.get(k) == Some(&desc)).then(|| args.offset(t.args[k] as isize))
        };
        let root = &*args.offset(t.args[t.dir as usize] as isize).add(t.root as usize).cast::<AtomsDirRoot>();
        let entries = root.entries.as_slice();
        let k = entries.partition_point(|e| e.first <= id);
        let e = entries.get(k.checked_sub(1)?)?;
        if id >= e.end {
            return None;
        }
        let base = arg(e.desc)?;
        let i = (id - e.from) as usize;
        let ends = base.add(e.ends as usize).cast::<u64>();
        let end = *ends.add(i) as usize;
        let start = if i == 0 { 0 } else { *ends.add(i - 1) as usize };
        Some(std::slice::from_raw_parts(base.add(e.strings as usize + start), end - start))
    }
}

/// The id of string `s`, whose hash (the store's, `hash_str`) is `h`, through the launch's arguments at `args` (host
/// addresses: `AtomsDir::host_args`), as the CLIF's `atoms_find` finds it with its caller's compare (the kernels'):
/// each run's table probed from `h`, a slot of `h`'s high half naming an atom of the run's own ids (`first..end`: one
/// its block holds below them is another run's) whose string is `s`, each in the run's block: the argument whose stable
/// id is the entry's. Id 0 is the empty string; `None` where no run holds `s`.
///
/// # Safety
/// As `str_in`'s.
pub unsafe fn find_through(args: *const u8, s: &[u8], h: u64) -> Option<u32> {
    if s.is_empty() {
        return Some(0);
    }
    // SAFETY: the caller's: a live record, its arguments in this address space, the directory's blocks mapped.
    unsafe {
        let t = &*args.cast::<AtomsArgs>();
        let arg = |desc: u32| {
            let ids = &t.ids[..t.n as usize];
            let k = ids.partition_point(|&d| d < desc);
            (ids.get(k) == Some(&desc)).then(|| args.offset(t.args[k] as isize))
        };
        let root = &*args.offset(t.args[t.dir as usize] as isize).add(t.root as usize).cast::<AtomsDirRoot>();
        let tag = h & 0xFFFF_FFFF_0000_0000;
        for e in root.entries.as_slice() {
            let slots = u64::from(e.slots);
            let Some(base) = arg(e.desc) else { continue };
            if slots == 0 {
                continue;
            }
            let (table, ends) = (base.add(e.table as usize).cast::<u64>(), base.add(e.ends as usize).cast::<u64>());
            let mut i = h & (slots - 1);
            for _ in 0..slots {
                let v = *table.add(i as usize);
                if v == 0 {
                    break;
                }
                let id = (v as u32).wrapping_sub(1);
                if v & 0xFFFF_FFFF_0000_0000 == tag && id >= e.first && id < e.end {
                    let k = (id - e.from) as usize;
                    let (start, end) = (if k == 0 { 0 } else { *ends.add(k - 1) as usize }, *ends.add(k) as usize);
                    if std::slice::from_raw_parts(base.add(e.strings as usize + start), end - start) == s {
                        return Some(id);
                    }
                }
                i = (i + 1) & (slots - 1);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    /// Decision 0478 step 2b: an id's string read through the directory, by the host's read (`str_in`, the CLIF's
    /// `atoms_at` and `atoms_len` written directly), is the block's own, in every run: a run that begins past its block's first id (a pending
    /// block whose first atoms were stored since) too. Id 0 is the empty string; an id no run holds has none.
    #[test]
    fn the_directorys_reads_find_every_atom_where_its_block_holds_it() {
        use super::{AtomRead, AtomsDir, AtomsW, str_in};
        let a = AtomsW::anon(1, 8, 256);
        let mut a = a;
        let words: Vec<String> = (0..20).map(|k| format!("w{k}{}", "z".repeat(k))).collect();
        for w in &words[..8] {
            a.append(w, k_hash(w)).unwrap();
        }
        let mut b = AtomsW::anon(9, 16, 1024);
        for w in &words[8..] {
            b.append(w, k_hash(w)).unwrap();
        }
        // The second block's first three atoms stored since: its run begins at 12, the store's run covers them.
        let runs = [(a.run_from(1), a.block().clone()), (b.run_from(12), b.block().clone())];
        let dir = AtomsDir::new(&runs);
        let t = dir.host_args().unwrap();
        for id in 1..=20u32 {
            // SAFETY: a host table of a directory over blocks that live here.
            let got = unsafe { str_in(t.root(), id) };
            let want = if (9..12).contains(&id) { None } else { [&a, &b].iter().find_map(|w| w.str(id)).map(str::as_bytes) };
            assert_eq!(got, want, "id {id}");
        }
        // SAFETY: as above.
        unsafe {
            assert_eq!(str_in(t.root(), 0), Some(&b""[..]), "id 0, the empty string");
            assert_eq!(str_in(t.root(), 21), None, "past every run");
        }
        // Found by its hash in its run's table: an id the directory holds; one below the second run's first, in its
        // block but no run's, is not found there; a string no block holds is not.
        for (k, w) in words.iter().enumerate() {
            let id = k as u32 + 1;
            // SAFETY: as above.
            let got = unsafe { super::find_through(t.root(), w.as_bytes(), k_hash(w)) };
            assert_eq!(got, (!(9..12).contains(&id)).then_some(id), "{w}");
        }
        // SAFETY: as above.
        unsafe {
            assert_eq!(super::find_through(t.root(), b"", 0), Some(0), "the empty string");
            assert_eq!(super::find_through(t.root(), b"absent", k_hash("absent")), None);
            assert_eq!(super::find_through(t.root(), b"w1", k_hash("w2z")), None, "another string's hash");
        }
    }

    /// A launch passes the directory and every block it names among its 30 arguments: a directory over 29 atoms
    /// blocks binds, and each entry's block is found by its descriptor's stable id; one over 30 is a launch error
    /// that names the count.
    #[test]
    fn a_directory_past_a_launchs_blocks_is_a_launch_error() {
        use super::{AtomsDir, AtomsW, LAUNCH_BLOCKS, str_in};
        let made: Vec<AtomsW> = (0..LAUNCH_BLOCKS as u32)
            .map(|k| {
                let mut w = AtomsW::anon(k + 1, 1, 64);
                w.append(&format!("a{k}"), k_hash(&format!("a{k}"))).unwrap();
                w
            })
            .collect();
        let runs = |n: usize| made[..n].iter().map(|w| (w.run_from(0), w.block().clone())).collect::<Vec<_>>();
        let fits = AtomsDir::new(&runs(LAUNCH_BLOCKS - 1));
        let t = fits.host_args().unwrap();
        // SAFETY: a host table of a directory over blocks that live here.
        assert_eq!(unsafe { str_in(t.root(), LAUNCH_BLOCKS as u32 - 1) }, Some(&b"a28"[..]), "the last block's atom");
        let past = AtomsDir::new(&runs(LAUNCH_BLOCKS)).host_args().err().expect("past the launch's blocks");
        assert!(past.contains("over 31 blocks") && past.contains("at most 30"), "{past}");
    }

    fn k_hash(s: &str) -> u64 {
        s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, c| (h ^ u64::from(c)).wrapping_mul(0x100_0000_01b3))
    }

    use super::*;

    fn h(s: &str) -> u64 {
        use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};
        BuildHasherDefault::<DefaultHasher>::default().hash_one(s)
    }

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("rerail-atoms-block-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        crate::mapped::make_folder(&d).unwrap();
        d
    }

    /// A-R1: appended strings are found, read and hashed; one not appended is not found; a full block refuses.
    #[test]
    fn an_atoms_block_finds_what_it_holds() {
        let d = dir("finds");
        let mut w = AtomsW::create(&d.join("d0.blk"), 10, 7, 1000, 1 << 16, Flush::Synced).unwrap();
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
        // Two strings of one hash (a collision): each found by comparing the string at the slot.
        let mut c = AtomsW::create(&d.join("d2.blk"), 0, 7, 16, 1024, Flush::Synced).unwrap();
        assert_eq!((c.append("x", 42), c.append("y", 42)), (Some(0), Some(1)));
        assert_eq!((c.find("y", 42), c.find("x", 42), c.find("z", 42)), (Some(1), Some(0), None));
        // Room for bytes runs out too.
        let mut small = AtomsW::create(&d.join("d1.blk"), 0, 7, 100, 8, Flush::Synced).unwrap();
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
        let mut w = AtomsW::create(&path, 0, 7, 64, 4096, Flush::Synced).unwrap();
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
        let mut again = AtomsW::reopen(&path, 3, Flush::Synced).unwrap();
        assert_eq!(again.end(), 3);
        assert_eq!(again.find("d", h("d")), None, "an append past the committed end is taken back");
        assert_eq!(again.append("e", h("e")), Some(3), "its id is the next writer's");
        assert_eq!(again.find("e", h("e")), Some(3));
        // A view at the old end still finds what it read before (the stale slot of "d" is passed over).
        let old = AtomsV::open(&path, 3).unwrap();
        assert_eq!(old.find("c", h("c")), Some(2));
        assert_eq!(old.find("e", h("e")), None);
        for (s, id) in [("a", 0), ("b", 1), ("c", 2)] {
            assert_eq!(again.find(s, h(s)), Some(id));
        }
        // (Windows cuts no file a view maps: the readers' views go first.)
        drop((v, old));
        // Frozen: the file is what it holds, and still reads. A mapped file (Unix, Windows) is the whole reservation
        // until then, so it shrinks; elsewhere the file was written only to the block's end (`sync`).
        again.sync().unwrap();
        let before = std::fs::metadata(&path).unwrap().len();
        again.freeze().unwrap();
        let after = std::fs::metadata(&path).unwrap().len();
        let frozen = AtomsV::open(&path, 4).unwrap();
        assert_eq!(after, frozen._block.header().length, "the frozen file is its header's length");
        #[cfg(any(unix, windows))]
        assert!(after < before, "the mapping's unwritten tail is given back");
        let _ = before;
        assert_eq!(frozen.find("e", h("e")), Some(3));
        assert_eq!(frozen.str(0), Some("a"));
        assert!(AtomsW::reopen(&path, 4, Flush::Synced).is_err(), "a frozen block takes no appends");
        let _ = std::fs::remove_dir_all(&d);
    }
}
